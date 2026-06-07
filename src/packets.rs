use anyhow::Result;
use chrono::{Duration, Local, NaiveDateTime, TimeZone, Timelike};
use serde_json::{json, Value};

use crate::{
    models::{
        GeneratedPacket, ReplayConfig, TrackPoint, Venue, DEFAULT_TARGET, LOCAL_TIME_FORMAT,
        RUN_COUNT_TARGET, SIGN_SALT,
    },
    track::generate_track_points,
};

pub fn parse_local_time(value: &str) -> Result<NaiveDateTime> {
    Ok(NaiveDateTime::parse_from_str(value, LOCAL_TIME_FORMAT)?)
}

pub fn format_local_time(value: NaiveDateTime) -> String {
    value.format(LOCAL_TIME_FORMAT).to_string()
}

pub fn calculate_sign(uid: &str, login_name: &str, timestamp: i64) -> String {
    let digest = md5::compute(format!("{uid}{login_name}{timestamp}{SIGN_SALT}"));
    format!("{digest:x}")
}

pub fn normalize_authorization(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    if value.to_ascii_lowercase().starts_with("bearer ") {
        Some(value.to_owned())
    } else {
        Some(format!("Bearer {value}"))
    }
}

pub fn serialize_body(body: &Value, pretty: bool) -> Result<Vec<u8>> {
    Ok(if pretty {
        serde_json::to_vec_pretty(body)?
    } else {
        serde_json::to_vec(body)?
    })
}

pub fn format_distance_km(meters: f64) -> String {
    let text = format!("{}", (meters * 1_000_000.0).round() / 1_000_000.0 / 1000.0);
    let Some((whole, fraction)) = text.split_once('.') else {
        return format!("{text}.00");
    };
    format!("{whole}.{fraction:0<2}")
        .chars()
        .take(whole.len() + 3)
        .collect()
}

pub fn chunk_points(points: &[TrackPoint], packet_seconds: usize) -> Vec<Vec<TrackPoint>> {
    let packet_seconds = packet_seconds.max(1);
    points
        .chunks(packet_seconds)
        .map(|chunk| chunk.to_vec())
        .collect()
}

fn track_point_to_body(point: &TrackPoint) -> Value {
    json!({
        "latitude": point.latitude,
        "longitude": point.longitude,
        "time": point.time,
        "speed": point.speed,
        "altitude": point.altitude,
        "attribute1": point.attribute1,
        "dist": point.dist,
    })
}

pub fn build_packets(
    venue: &Venue,
    config: &ReplayConfig,
    uid: &str,
) -> Result<Vec<GeneratedPacket>> {
    let points = generate_track_points(venue, config)?;
    let chunks = chunk_points(&points, config.packet_seconds);
    let total_packets = chunks.len() + 1;
    let mut packets = Vec::with_capacity(total_packets);
    let mut cumulative_distance = 0.0;
    let mut cumulative_steps = 0_i64;
    let mut avg_frequency = 0.0;
    let mut all_altitudes = Vec::new();
    let stride_m = 1.68;
    let mut final_body = None;
    let mut final_time = None;

    for (zero_index, chunk) in chunks.iter().enumerate() {
        let index = zero_index + 1;
        let chunk_distance = chunk.iter().map(|point| point.dist).sum::<f64>();
        cumulative_distance += chunk_distance;
        all_altitudes.extend(chunk.iter().map(|point| point.altitude));
        let elapsed_minutes =
            (index as f64 * config.packet_seconds as f64 / 60.0).min(config.duration_minutes);
        let avg_speed = cumulative_distance / (elapsed_minutes * 60.0).max(1.0);
        avg_frequency = (avg_speed / stride_m * 60.0).clamp(108.0, 123.0);
        cumulative_steps = (avg_frequency * elapsed_minutes).round() as i64;
        let last_point = chunk
            .last()
            .ok_or_else(|| anyhow::anyhow!("generated an empty packet chunk"))?;
        let last_time = parse_local_time(&last_point.time)?;
        let timestamp = local_timestamp_millis(last_time);
        let body = json!({
            "uid": uid,
            "details": chunk.iter().map(track_point_to_body).collect::<Vec<_>>(),
            "standardPace": &config.standard_pace,
            "resultKm": &config.result_km,
            "timestamp": timestamp,
            "sign": calculate_sign(uid, &config.login_name, timestamp),
            "runStatus": &config.run_status,
            "loginName": &config.login_name,
            "semesterId": &config.semester_id,
            "semesterName": &config.semester_name,
            "totalKm": format_distance_km(cumulative_distance),
            "avgFrequency": avg_frequency,
            "altitudeMin": all_altitudes.iter().copied().fold(f64::INFINITY, f64::min),
            "altitudeMax": all_altitudes.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            "stopTime": 0,
            "timeFrom": format_local_time(config.start_time),
            "sex": &config.sex,
            "venueId": &venue.venue_id,
            "venueName": &venue.venue_name,
            "stepNumber": cumulative_steps,
        });
        final_body = Some(body.clone());
        final_time = Some(last_time);
        packets.push(GeneratedPacket {
            index,
            total: total_packets,
            method: "POST".to_owned(),
            target: DEFAULT_TARGET.to_owned(),
            headers: default_headers(config, &body)?,
            body,
            scheduled_at: last_time,
        });
    }

    if let (Some(final_body), Some(final_time)) = (final_body, final_time) {
        let finish_time = final_time + Duration::seconds(6);
        let finish_timestamp = local_timestamp_millis(finish_time);
        let finish_body = json!({
            "uid": uid,
            "details": [],
            "standardPace": &config.standard_pace,
            "resultKm": &config.result_km,
            "timestamp": finish_timestamp,
            "sign": calculate_sign(uid, &config.login_name, finish_timestamp),
            "runStatus": &config.run_status,
            "loginName": &config.login_name,
            "semesterId": &config.semester_id,
            "semesterName": &config.semester_name,
            "totalKm": final_body["totalKm"],
            "avgFrequency": avg_frequency,
            "altitudeMin": final_body["altitudeMin"],
            "altitudeMax": final_body["altitudeMax"],
            "stopTime": 0,
            "timeFrom": final_body["timeFrom"],
            "sex": &config.sex,
            "venueId": &venue.venue_id,
            "venueName": &venue.venue_name,
            "stepNumber": cumulative_steps,
            "finish": 1,
            "timeTo": format_local_time(final_time),
        });
        packets.push(GeneratedPacket {
            index: total_packets,
            total: total_packets,
            method: "POST".to_owned(),
            target: DEFAULT_TARGET.to_owned(),
            headers: default_headers(config, &finish_body)?,
            body: finish_body,
            scheduled_at: finish_time,
        });
    }
    Ok(packets)
}

pub fn build_run_count_packet(config: &ReplayConfig) -> Result<GeneratedPacket> {
    let body = json!({
        "loginName": &config.login_name,
        "semesterId": &config.semester_id,
        "runStatus": &config.run_status,
    });
    Ok(GeneratedPacket {
        index: 1,
        total: 1,
        method: "POST".to_owned(),
        target: RUN_COUNT_TARGET.to_owned(),
        headers: default_headers(config, &body)?,
        body,
        scheduled_at: Local::now()
            .naive_local()
            .with_nanosecond(0)
            .unwrap_or_else(|| Local::now().naive_local()),
    })
}

pub fn default_run_start(
    now: NaiveDateTime,
    duration_minutes: f64,
    packet_seconds: usize,
) -> NaiveDateTime {
    let duration_seconds = 1.max((duration_minutes * 60.0).round() as i64);
    let first_packet_seconds = packet_seconds.max(1).min(duration_seconds as usize) as i64;
    now - Duration::seconds(first_packet_seconds)
}

pub fn default_headers(config: &ReplayConfig, body: &Value) -> Result<Vec<(String, String)>> {
    let body_bytes = serialize_body(body, false)?;
    let mut headers = vec![
        ("Connection".to_owned(), "keep-alive".to_owned()),
        ("Content-Length".to_owned(), body_bytes.len().to_string()),
        ("content-type".to_owned(), "application/json".to_owned()),
        ("Accept-Encoding".to_owned(), "gzip, deflate".to_owned()),
        ("Referer".to_owned(), config.referer.clone()),
    ];
    if let Some(authorization) = normalize_authorization(config.authorization.as_deref()) {
        headers.insert(2, ("Authorization".to_owned(), authorization));
    }
    if let Some(user_agent) = &config.user_agent {
        if !user_agent.trim().is_empty() {
            headers.push(("User-Agent".to_owned(), user_agent.clone()));
        }
    }
    Ok(headers)
}

pub fn rewrite_headers(
    headers: &[(String, String)],
    body_bytes: &[u8],
    redact_authorization: bool,
    authorization_override: Option<&str>,
    user_agent_override: Option<&str>,
) -> Vec<(String, String)> {
    let normalized_authorization = normalize_authorization(authorization_override);
    let mut result = Vec::new();
    let mut saw_content_length = false;
    let mut saw_authorization = false;
    let mut saw_user_agent = false;
    for (name, value) in headers {
        match name.to_ascii_lowercase().as_str() {
            "content-length" => {
                result.push((name.clone(), body_bytes.len().to_string()));
                saw_content_length = true;
            }
            "authorization" => {
                saw_authorization = true;
                let value = normalized_authorization.as_ref().unwrap_or(value);
                result.push((
                    name.clone(),
                    if redact_authorization {
                        "<redacted>".to_owned()
                    } else {
                        value.clone()
                    },
                ));
            }
            "user-agent" => {
                saw_user_agent = true;
                result.push((
                    name.clone(),
                    user_agent_override.unwrap_or(value).to_owned(),
                ));
            }
            _ => result.push((name.clone(), value.clone())),
        }
    }
    if !body_bytes.is_empty() && !saw_content_length {
        result.push(("Content-Length".to_owned(), body_bytes.len().to_string()));
    }
    if let Some(authorization) = normalized_authorization {
        if !saw_authorization {
            result.push((
                "Authorization".to_owned(),
                if redact_authorization {
                    "<redacted>".to_owned()
                } else {
                    authorization
                },
            ));
        }
    }
    if let Some(user_agent) = user_agent_override {
        if !saw_user_agent {
            result.push(("User-Agent".to_owned(), user_agent.to_owned()));
        }
    }
    result
}

pub fn build_http_text(
    packet: &GeneratedPacket,
    headers: &[(String, String)],
    body_bytes: &[u8],
) -> String {
    let mut lines = vec![format!("{} {} HTTP/1.1", packet.method, packet.target)];
    lines.extend(
        headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}")),
    );
    lines.push(String::new());
    if !body_bytes.is_empty() {
        lines.push(String::from_utf8_lossy(body_bytes).to_string());
    }
    lines.join("\r\n")
}

fn local_timestamp_millis(value: NaiveDateTime) -> i64 {
    match Local.from_local_datetime(&value) {
        chrono::LocalResult::Single(dt) => dt.timestamp_millis(),
        chrono::LocalResult::Ambiguous(dt, _) => dt.timestamp_millis(),
        chrono::LocalResult::None => value.and_utc().timestamp_millis(),
    }
}
