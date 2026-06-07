use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::models::Venue;

pub fn load_venues(path: impl AsRef<Path>) -> Result<Vec<Venue>> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let payload: Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    parse_venues(&payload)
}

pub fn parse_venues(payload: &Value) -> Result<Vec<Venue>> {
    let places = payload
        .get("healthRunningPlaces")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("venues JSON must contain a healthRunningPlaces list"))?;

    let mut venues = Vec::new();
    for (index, place) in places.iter().enumerate() {
        let Some(object) = place.as_object() else {
            continue;
        };
        let polygon = parse_polygon(place);
        if polygon.len() < 3 {
            continue;
        }
        venues.push(Venue {
            index,
            venue_id: value_to_string(object.get("venueId")).unwrap_or_default(),
            venue_name: value_to_string(object.get("venueName"))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| format!("Venue {}", index + 1)),
            campus_name: value_to_string(object.get("campusName")).unwrap_or_default(),
            open: object.get("open").and_then(Value::as_bool),
            polygon_lonlat: polygon,
            raw: place.clone(),
        });
    }

    if venues.is_empty() {
        anyhow::bail!("no usable venue polygons found");
    }
    Ok(venues)
}

pub fn parse_polygon(place: &Value) -> Vec<(f64, f64)> {
    let mut points = Vec::new();
    if let Some(entities) = place.get("coordinateEntityList").and_then(Value::as_array) {
        for item in entities {
            let lon = item.get("longitude").and_then(value_to_f64);
            let lat = item.get("latitude").and_then(value_to_f64);
            if let (Some(lon), Some(lat)) = (lon, lat) {
                points.push((lon, lat));
            }
        }
    }
    if !points.is_empty() {
        return strip_duplicate_close(points);
    }

    if let Some(locations) = place.get("locationList").and_then(Value::as_array) {
        for item in locations {
            let Some(text) = item.as_str() else {
                continue;
            };
            let Some((lon, lat)) = text.split_once(',') else {
                continue;
            };
            if let (Ok(lon), Ok(lat)) = (lon.trim().parse::<f64>(), lat.trim().parse::<f64>()) {
                points.push((lon, lat));
            }
        }
    }
    strip_duplicate_close(points)
}

fn strip_duplicate_close(mut points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    points
}

fn value_to_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
