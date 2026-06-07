use anyhow::Result;
use chrono::Duration;

use crate::{
    geometry::{
        ensure_ccw, line_intersection, lonlat_origin, point_in_polygon, polyline_length,
        project_lonlat, sample_closed_polyline, unproject_xy,
    },
    models::{ReplayConfig, TrackPoint, Venue, LOCAL_TIME_FORMAT},
    python_random::PythonRandom,
};

pub fn inward_offset_polygon(points: &[(f64, f64)], inset_m: f64) -> Result<Vec<(f64, f64)>> {
    let polygon = ensure_ccw(points);
    if inset_m <= 0.0 {
        return Ok(polygon);
    }

    let mut shifted_edges = Vec::new();
    for (a, b) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let length = dx.hypot(dy);
        if length <= 1e-9 {
            continue;
        }
        let nx = -dy / length;
        let ny = dx / length;
        shifted_edges.push((
            (a.0 + nx * inset_m, a.1 + ny * inset_m),
            (b.0 + nx * inset_m, b.1 + ny * inset_m),
        ));
    }

    let mut offset = Vec::new();
    for i in 0..shifted_edges.len() {
        let previous = shifted_edges[(i + shifted_edges.len() - 1) % shifted_edges.len()];
        let current = shifted_edges[i];
        offset.push(
            line_intersection(previous.0, previous.1, current.0, current.1).unwrap_or(current.0),
        );
    }

    if offset.len() >= 3
        && offset
            .iter()
            .all(|point| point_in_polygon(*point, &polygon))
    {
        return Ok(ensure_ccw(&offset));
    }
    radial_inset_polygon(&polygon, inset_m)
}

fn radial_inset_polygon(points: &[(f64, f64)], inset_m: f64) -> Result<Vec<(f64, f64)>> {
    let polygon = ensure_ccw(points);
    let cx = polygon.iter().map(|point| point.0).sum::<f64>() / polygon.len() as f64;
    let cy = polygon.iter().map(|point| point.1).sum::<f64>() / polygon.len() as f64;
    let candidate: Vec<_> = polygon
        .iter()
        .map(|(x, y)| {
            let dx = cx - x;
            let dy = cy - y;
            let distance = dx.hypot(dy);
            if distance <= 1e-9 {
                (*x, *y)
            } else {
                let movement = inset_m.min(distance * 0.85);
                (x + dx / distance * movement, y + dy / distance * movement)
            }
        })
        .collect();

    if candidate
        .iter()
        .all(|point| point_in_polygon(*point, &polygon))
    {
        return Ok(ensure_ccw(&candidate));
    }

    for scale in [0.75, 0.60, 0.45, 0.30, 0.20] {
        let scaled: Vec<_> = polygon
            .iter()
            .map(|(x, y)| (cx + (x - cx) * scale, cy + (y - cy) * scale))
            .collect();
        if scaled
            .iter()
            .all(|point| point_in_polygon(*point, &polygon))
        {
            return Ok(ensure_ccw(&scaled));
        }
    }
    anyhow::bail!("cannot build an internal track for this venue polygon")
}

fn track_inside_polygon(track: &[(f64, f64)], polygon: &[(f64, f64)]) -> bool {
    if track.len() < 3 {
        return false;
    }
    let length = polyline_length(track);
    let sample_count = track.len().max((length / 2.0) as usize + 1).min(600);
    (0..sample_count).all(|index| {
        let Ok(point) = sample_closed_polyline(track, length * index as f64 / sample_count as f64)
        else {
            return false;
        };
        point_in_polygon(point, polygon)
    })
}

fn stadium_track_candidate(
    points: &[(f64, f64)],
    polygon: &[(f64, f64)],
) -> Option<Vec<(f64, f64)>> {
    if points.len() < 4 {
        return None;
    }
    let cx = points.iter().map(|point| point.0).sum::<f64>() / points.len() as f64;
    let cy = points.iter().map(|point| point.1).sum::<f64>() / points.len() as f64;
    let xx = points.iter().map(|(x, _)| (x - cx).powi(2)).sum::<f64>() / points.len() as f64;
    let yy = points.iter().map(|(_, y)| (y - cy).powi(2)).sum::<f64>() / points.len() as f64;
    let xy = points.iter().map(|(x, y)| (x - cx) * (y - cy)).sum::<f64>() / points.len() as f64;
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    let cos_a = angle.cos();
    let sin_a = angle.sin();

    let to_local = |point: (f64, f64)| {
        let x = point.0 - cx;
        let y = point.1 - cy;
        (x * cos_a + y * sin_a, -x * sin_a + y * cos_a)
    };
    let to_world = |point: (f64, f64)| {
        let (x, y) = point;
        (cx + x * cos_a - y * sin_a, cy + x * sin_a + y * cos_a)
    };

    let mut local: Vec<_> = points.iter().copied().map(to_local).collect();
    let mut min_x = local
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let mut max_x = local
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut min_y = local
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let mut max_y = local
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut length = max_x - min_x;
    let mut width = max_y - min_y;
    let swapped = width > length;
    if swapped {
        local = local.iter().map(|(x, y)| (*y, *x)).collect();
        min_x = local
            .iter()
            .map(|point| point.0)
            .fold(f64::INFINITY, f64::min);
        max_x = local
            .iter()
            .map(|point| point.0)
            .fold(f64::NEG_INFINITY, f64::max);
        min_y = local
            .iter()
            .map(|point| point.1)
            .fold(f64::INFINITY, f64::min);
        max_y = local
            .iter()
            .map(|point| point.1)
            .fold(f64::NEG_INFINITY, f64::max);
        length = max_x - min_x;
        width = max_y - min_y;
    }

    if length <= 0.0 || width <= 0.0 || length / width < 1.18 {
        return None;
    }
    let radius = width / 2.0;
    let half_straight = length / 2.0 - radius;
    if half_straight <= radius * 0.2 {
        return None;
    }

    let world = |point: (f64, f64)| {
        if swapped {
            to_world((point.1, point.0))
        } else {
            to_world(point)
        }
    };
    let points_per_arc = usize::max(16, (std::f64::consts::PI * radius / 3.0) as usize);
    let mut candidate = Vec::new();
    candidate.push(world((-half_straight, -radius)));
    candidate.push(world((half_straight, -radius)));
    for step in 1..=points_per_arc {
        let theta = -std::f64::consts::FRAC_PI_2
            + std::f64::consts::PI * step as f64 / points_per_arc as f64;
        candidate.push(world((
            half_straight + radius * theta.cos(),
            radius * theta.sin(),
        )));
    }
    candidate.push(world((-half_straight, radius)));
    for step in 1..=points_per_arc {
        let theta = std::f64::consts::FRAC_PI_2
            + std::f64::consts::PI * step as f64 / points_per_arc as f64;
        candidate.push(world((
            -half_straight + radius * theta.cos(),
            radius * theta.sin(),
        )));
    }
    let candidate = ensure_ccw(&candidate);
    track_inside_polygon(&candidate, polygon).then_some(candidate)
}

fn chaikin_smooth_closed(points: &[(f64, f64)], iterations: usize) -> Vec<(f64, f64)> {
    let mut smoothed = ensure_ccw(points);
    for _ in 0..iterations {
        let mut next = Vec::with_capacity(smoothed.len() * 2);
        for (a, b) in smoothed
            .iter()
            .zip(smoothed.iter().cycle().skip(1))
            .take(smoothed.len())
        {
            next.push((0.75 * a.0 + 0.25 * b.0, 0.75 * a.1 + 0.25 * b.1));
            next.push((0.25 * a.0 + 0.75 * b.0, 0.25 * a.1 + 0.75 * b.1));
        }
        smoothed = next;
    }
    ensure_ccw(&smoothed)
}

fn smooth_track_xy(points: &[(f64, f64)], polygon: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if let Some(stadium) = stadium_track_candidate(points, polygon) {
        return stadium;
    }
    for iterations in [5, 4, 3, 2] {
        let smoothed = chaikin_smooth_closed(points, iterations);
        if track_inside_polygon(&smoothed, polygon) {
            return smoothed;
        }
    }
    ensure_ccw(points)
}

pub fn build_internal_track_lonlat(venue: &Venue, inset_m: f64) -> Result<Vec<(f64, f64)>> {
    let origin = lonlat_origin(&venue.polygon_lonlat);
    let polygon_xy = project_lonlat(&venue.polygon_lonlat, origin);
    let inset_xy = inward_offset_polygon(&polygon_xy, inset_m)?;
    let track_xy = smooth_track_xy(&inset_xy, &polygon_xy);
    Ok(unproject_xy(&track_xy, origin))
}

fn bounded_jitter_xy(index: usize, phase_a: f64, phase_b: f64) -> (f64, f64) {
    let mut x =
        0.52 * (index as f64 / 13.0 + phase_a).sin() + 0.28 * (index as f64 / 5.1 + phase_b).sin();
    let mut y =
        0.48 * (index as f64 / 11.0 + phase_b).cos() + 0.30 * (index as f64 / 6.7 + phase_a).sin();
    let length = x.hypot(y);
    if length > 1.0 {
        x /= length;
        y /= length;
    }
    (x, y)
}

fn jitter_inside_polygon(
    point: (f64, f64),
    jitter: (f64, f64),
    polygon: &[(f64, f64)],
) -> (f64, f64) {
    for scale in [1.0, 0.75, 0.5, 0.25] {
        let candidate = (point.0 + jitter.0 * scale, point.1 + jitter.1 * scale);
        if point_in_polygon(candidate, polygon) {
            return candidate;
        }
    }
    point
}

pub fn generate_track_points(venue: &Venue, config: &ReplayConfig) -> Result<Vec<TrackPoint>> {
    let origin = lonlat_origin(&venue.polygon_lonlat);
    let venue_polygon_xy = project_lonlat(&venue.polygon_lonlat, origin);
    let track_xy = smooth_track_xy(
        &inward_offset_polygon(&venue_polygon_xy, config.inset_m)?,
        &venue_polygon_xy,
    );
    let loop_length = polyline_length(&track_xy);
    if loop_length <= 0.0 {
        anyhow::bail!("selected venue produced a zero-length track");
    }

    let duration_seconds = 1.max((config.duration_minutes * 60.0).round() as usize);
    let target_meters = 1.0_f64.max(config.track_km * 1000.0);
    let mut rng = PythonRandom::new(config.seed);
    let mut raw_weights = Vec::with_capacity(duration_seconds);
    for i in 0..duration_seconds {
        let wave = 1.0 + 0.12 * (i as f64 / 17.0).sin() + 0.05 * (i as f64 / 5.3).sin();
        raw_weights.push(0.55_f64.max(wave + rng.uniform(-0.08, 0.08)));
    }
    let scale = target_meters / raw_weights.iter().sum::<f64>();
    let mut step_distances: Vec<f64> = raw_weights.iter().map(|weight| weight * scale).collect();
    let correction = target_meters - step_distances.iter().sum::<f64>();
    if let Some(last) = step_distances.last_mut() {
        *last += correction;
    }

    let altitude_base = -4.5 + rng.uniform(-1.5, 1.5);
    let altitude_phase_a = rng.random() * std::f64::consts::TAU;
    let altitude_phase_b = rng.random() * std::f64::consts::TAU;
    let jitter_phase_a = rng.random() * std::f64::consts::TAU;
    let jitter_phase_b = rng.random() * std::f64::consts::TAU;
    let altitude_offsets: Vec<f64> = (0..step_distances.len())
        .map(|_| rng.uniform(-0.9, 0.9))
        .collect();

    let mut points = build_points_from_distances(
        &step_distances,
        &track_xy,
        &venue_polygon_xy,
        origin,
        config,
        altitude_base,
        altitude_phase_a,
        altitude_phase_b,
        jitter_phase_a,
        jitter_phase_b,
        &altitude_offsets,
    )?;
    let calibration_target_meters = target_meters + 0.75;
    for _ in 0..6 {
        let actual_meters = points.iter().map(|point| point.dist).sum::<f64>();
        if actual_meters <= 0.0 {
            break;
        }
        let ratio = calibration_target_meters / actual_meters;
        if (1.0 - ratio).abs() < 0.00005 {
            break;
        }
        for distance in &mut step_distances {
            *distance *= ratio;
        }
        points = build_points_from_distances(
            &step_distances,
            &track_xy,
            &venue_polygon_xy,
            origin,
            config,
            altitude_base,
            altitude_phase_a,
            altitude_phase_b,
            jitter_phase_a,
            jitter_phase_b,
            &altitude_offsets,
        )?;
    }
    Ok(points)
}

#[allow(clippy::too_many_arguments)]
fn build_points_from_distances(
    distances: &[f64],
    track_xy: &[(f64, f64)],
    venue_polygon_xy: &[(f64, f64)],
    origin: (f64, f64),
    config: &ReplayConfig,
    altitude_base: f64,
    altitude_phase_a: f64,
    altitude_phase_b: f64,
    jitter_phase_a: f64,
    jitter_phase_b: f64,
    altitude_offsets: &[f64],
) -> Result<Vec<TrackPoint>> {
    let mut points = Vec::with_capacity(distances.len());
    let mut cumulative = 0.0;
    let start_xy = sample_closed_polyline(track_xy, 0.0)?;
    let mut previous_xy = jitter_inside_polygon(
        start_xy,
        bounded_jitter_xy(0, jitter_phase_a, jitter_phase_b),
        venue_polygon_xy,
    );
    for (zero_index, distance) in distances.iter().enumerate() {
        let index = zero_index + 1;
        cumulative += distance;
        let (mut x, mut y) = sample_closed_polyline(track_xy, cumulative)?;
        if !point_in_polygon((x, y), venue_polygon_xy) {
            anyhow::bail!("generated track point escaped venue polygon");
        }
        (x, y) = jitter_inside_polygon(
            (x, y),
            bounded_jitter_xy(index, jitter_phase_a, jitter_phase_b),
            venue_polygon_xy,
        );
        let (lon, lat) = unproject_xy(&[(x, y)], origin)[0];
        let timestamp = config.start_time + Duration::seconds(index as i64);
        let altitude = altitude_base
            + 7.2 * (index as f64 / 96.0 + altitude_phase_a).sin()
            + 4.1 * (index as f64 / 41.0 + altitude_phase_b).sin()
            + altitude_offsets[zero_index];
        let actual_distance = (x - previous_xy.0).hypot(y - previous_xy.1);
        points.push(TrackPoint {
            latitude: lat,
            longitude: lon,
            time: timestamp.format(LOCAL_TIME_FORMAT).to_string(),
            speed: actual_distance,
            altitude,
            attribute1: 0,
            dist: actual_distance,
        });
        previous_xy = (x, y);
    }
    Ok(points)
}
