use std::{fs, path::Path, time::Duration};

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use url::Url;

use crate::{
    models::{GeneratedPacket, PRODUCTION_HOST},
    packets::{normalize_authorization, serialize_body},
};

pub const DEFAULT_VENUE_PATH: &str = "/api/public/tongji/requestVenue";

pub struct FetchVenuesOptions<'a> {
    pub base_url: &'a str,
    pub authorization: Option<&'a str>,
    pub out: &'a Path,
    pub path: &'a str,
    pub timeout_seconds: f64,
    pub longitude: Option<&'a str>,
    pub latitude: Option<&'a str>,
    pub open_type: Option<&'a str>,
    pub token_query: bool,
}

pub struct SendResult {
    pub log_text: String,
    pub body_text: String,
    pub status: u16,
}

pub fn token_only(authorization: Option<&str>) -> Option<String> {
    let authorization = authorization?.trim();
    if authorization.is_empty() {
        return None;
    }
    Some(
        authorization
            .strip_prefix("Bearer ")
            .or_else(|| authorization.strip_prefix("bearer "))
            .unwrap_or(authorization)
            .trim()
            .to_owned(),
    )
}

pub fn build_url(base_url: &str, path: &str, params: &[(String, String)]) -> Result<String> {
    let mut base = base_url.trim().trim_end_matches('/').to_owned();
    let mut endpoint = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let parsed_base = Url::parse(&base).context("base URL must be an absolute http(s) URL")?;
    if parsed_base
        .path()
        .trim_end_matches('/')
        .ends_with("/msports")
        && endpoint.starts_with("/msports/")
    {
        endpoint = endpoint.trim_start_matches("/msports").to_owned();
    }
    base.push_str(&endpoint);
    let mut url = Url::parse(&base)?;
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in params {
            pairs.append_pair(key, value);
        }
    }
    Ok(url.to_string())
}

pub fn validate_non_production_base_url(base_url: &str) -> Result<Url> {
    let url = Url::parse(base_url.trim()).context("Base URL must be an http(s) URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        anyhow::bail!("Base URL must be an http(s) URL");
    }
    if url.host_str() == Some(PRODUCTION_HOST) {
        anyhow::bail!("Refusing to send to production host: {PRODUCTION_HOST}");
    }
    Ok(url)
}

pub fn fetch_venues(options: FetchVenuesOptions<'_>) -> Result<usize> {
    let authorization = normalize_authorization(options.authorization);
    let mut params = Vec::new();
    if let Some(longitude) = present_param(options.longitude) {
        params.push(("longitude".to_owned(), longitude.to_owned()));
    }
    if let Some(latitude) = present_param(options.latitude) {
        params.push(("latitude".to_owned(), latitude.to_owned()));
    }
    if let Some(open_type) = present_param(options.open_type) {
        params.push(("openType".to_owned(), open_type.to_owned()));
    }
    if options.token_query {
        if let Some(token) = token_only(authorization.as_deref()) {
            params.push(("token".to_owned(), token));
        }
    }
    let url = build_url(options.base_url, options.path, &params)?;
    let timeout = Duration::from_secs_f64(options.timeout_seconds.max(0.001));
    let client = Client::builder().timeout(timeout).build()?;
    let mut request = client
        .get(url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "identity")
        .header("User-Agent", "health-run-venue-fetcher/1.0");
    if let Some(authorization) = &authorization {
        request = request.header("Authorization", authorization);
    }
    let response: Value = request.send()?.error_for_status()?.json()?;
    let health_places = extract_health_running_places(&response);
    let health_place_count = health_places.len();
    let query: serde_json::Map<String, Value> = params
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                Value::String(if key == "token" {
                    "<redacted>".to_owned()
                } else {
                    value.clone()
                }),
            )
        })
        .collect();
    let payload = json!({
        "fetchedAt": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "baseUrl": options.base_url,
        "path": options.path,
        "query": query,
        "authorizationProvided": authorization.is_some(),
        "healthRunningPlaceCount": health_place_count,
        "healthRunningPlaces": health_places,
        "rawResponse": response,
    });
    let out = options.out;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out, serde_json::to_string_pretty(&payload)? + "\n")?;
    Ok(health_place_count)
}

pub fn send_packet(base_url: &str, packet: &GeneratedPacket) -> Result<String> {
    Ok(send_packet_result(base_url, packet)?.log_text)
}

pub fn send_packet_result(base_url: &str, packet: &GeneratedPacket) -> Result<SendResult> {
    let base_url = validate_non_production_base_url(base_url)?;
    let url = full_url(base_url, &packet.target)?;
    let body = serialize_body(&packet.body, false)?;
    let client = Client::builder().build()?;
    let method = packet.method.parse::<reqwest::Method>()?;
    let mut request = client.request(method.clone(), url.clone()).body(body);
    for (name, value) in &packet.headers {
        if !matches!(
            name.to_ascii_lowercase().as_str(),
            "host" | "content-length"
        ) {
            request = request.header(name.as_str(), value.as_str());
        }
    }
    let response = request.send()?;
    let status = response.status();
    let headers = response.headers().clone();
    let text = response.text().unwrap_or_default();
    Ok(SendResult {
        log_text: format!(
            "\n### {} {}\nHTTP {}\n{:?}\n{}\n",
            method, url, status, headers, text
        ),
        body_text: text,
        status: status.as_u16(),
    })
}

pub fn full_url(mut base_url: Url, target: &str) -> Result<Url> {
    let mut path = target.to_owned();
    if base_url.path().trim_end_matches('/').ends_with("/msports") && path.starts_with("/msports/")
    {
        path = path.trim_start_matches("/msports").to_owned();
    }
    let joined = format!(
        "{}/{}",
        base_url.as_str().trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    base_url = Url::parse(&joined)?;
    Ok(base_url)
}

pub fn extract_health_running_places(response: &Value) -> Vec<Value> {
    let mut places = Vec::new();
    let Some(data) = response.get("data").and_then(Value::as_array) else {
        return places;
    };
    for semester_or_group in data {
        let Some(running_points) = semester_or_group
            .get("runningPointList")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for place in running_points {
            let open_type = place
                .get("openType")
                .map(value_as_string)
                .unwrap_or_default();
            let location_list = place.get("locationList").and_then(Value::as_array);
            if !open_type.contains('0') || location_list.map_or(true, |items| items.is_empty()) {
                continue;
            }
            places.push(summarize_place(place, semester_or_group));
        }
    }
    places
}

fn summarize_place(place: &Value, parent: &Value) -> Value {
    json!({
        "venueId": place.get("venueId").cloned().unwrap_or(Value::Null),
        "venueName": place.get("venueName").cloned().unwrap_or(Value::Null),
        "campusName": place.get("campusName").cloned().or_else(|| parent.get("campusName").cloned()).unwrap_or(Value::Null),
        "open": place.get("open").cloned().unwrap_or(Value::Null),
        "openType": place.get("openType").cloned().unwrap_or(Value::Null),
        "openRunTimeListNew": place.get("openRunTimeListNew").cloned().unwrap_or(Value::Null),
        "locationList": place.get("locationList").cloned().unwrap_or(Value::Null),
        "coordinateEntityList": place.get("coordinateEntityList").cloned().unwrap_or(Value::Null),
        "raw": place.clone(),
    })
}

fn value_as_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => String::new(),
    }
}

fn present_param(value: Option<&str>) -> Option<&str> {
    let value = value?.trim();
    (!value.is_empty()).then_some(value)
}
