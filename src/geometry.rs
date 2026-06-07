use crate::models::EARTH_RADIUS_M;

pub fn haversine_m(a_lat: f64, a_lon: f64, b_lat: f64, b_lon: f64) -> f64 {
    let lat1 = a_lat.to_radians();
    let lat2 = b_lat.to_radians();
    let d_lat = lat2 - lat1;
    let d_lon = (b_lon - a_lon).to_radians();
    let h = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);
    EARTH_RADIUS_M * 2.0 * h.sqrt().atan2((1.0 - h).sqrt())
}

pub fn lonlat_origin(points: &[(f64, f64)]) -> (f64, f64) {
    let (lon_sum, lat_sum) = points
        .iter()
        .fold((0.0, 0.0), |acc, point| (acc.0 + point.0, acc.1 + point.1));
    (lon_sum / points.len() as f64, lat_sum / points.len() as f64)
}

pub fn project_lonlat(points: &[(f64, f64)], origin: (f64, f64)) -> Vec<(f64, f64)> {
    let (lon0, lat0) = origin;
    let scale_x = 111_320.0 * lat0.to_radians().cos();
    points
        .iter()
        .map(|(lon, lat)| ((lon - lon0) * scale_x, (lat - lat0) * 111_320.0))
        .collect()
}

pub fn unproject_xy(points: &[(f64, f64)], origin: (f64, f64)) -> Vec<(f64, f64)> {
    let (lon0, lat0) = origin;
    let scale_x = 111_320.0 * lat0.to_radians().cos();
    points
        .iter()
        .map(|(x, y)| (lon0 + x / scale_x, lat0 + y / 111_320.0))
        .collect()
}

pub fn signed_area(points: &[(f64, f64)]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|((x1, y1), (x2, y2))| x1 * y2 - x2 * y1)
        .sum::<f64>()
        / 2.0
}

pub fn ensure_ccw(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if signed_area(points) > 0.0 {
        points.to_vec()
    } else {
        points.iter().rev().copied().collect()
    }
}

pub fn point_in_polygon(point: (f64, f64), polygon: &[(f64, f64)]) -> bool {
    let (x, y) = point;
    let mut inside = false;
    for index in 0..polygon.len() {
        let (x1, y1) = polygon[index];
        let (x2, y2) = polygon[(index + 1) % polygon.len()];
        if point_on_segment(point, (x1, y1), (x2, y2), 1e-7) {
            return true;
        }
        let crosses = (y1 > y) != (y2 > y);
        if crosses {
            let denominator = if (y2 - y1).abs() <= 1e-12 {
                1e-12
            } else {
                y2 - y1
            };
            let x_at_y = (x2 - x1) * (y - y1) / denominator + x1;
            if x_at_y >= x {
                inside = !inside;
            }
        }
    }
    inside
}

pub fn point_on_segment(point: (f64, f64), a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    let (px, py) = point;
    let (ax, ay) = a;
    let (bx, by) = b;
    let cross = (px - ax) * (by - ay) - (py - ay) * (bx - ax);
    if cross.abs() > tolerance {
        return false;
    }
    let dot = (px - ax) * (px - bx) + (py - ay) * (py - by);
    dot <= tolerance
}

pub fn line_intersection(
    a1: (f64, f64),
    a2: (f64, f64),
    b1: (f64, f64),
    b2: (f64, f64),
) -> Option<(f64, f64)> {
    let (x1, y1) = a1;
    let (x2, y2) = a2;
    let (x3, y3) = b1;
    let (x4, y4) = b2;
    let denom = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
    if denom.abs() < 1e-9 {
        return None;
    }
    let px = ((x1 * y2 - y1 * x2) * (x3 - x4) - (x1 - x2) * (x3 * y4 - y3 * x4)) / denom;
    let py = ((x1 * y2 - y1 * x2) * (y3 - y4) - (y1 - y2) * (x3 * y4 - y3 * x4)) / denom;
    Some((px, py))
}

pub fn polyline_length(points: &[(f64, f64)]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| (b.0 - a.0).hypot(b.1 - a.1))
        .sum()
}

pub fn sample_closed_polyline(
    points: &[(f64, f64)],
    distance_m: f64,
) -> anyhow::Result<(f64, f64)> {
    let loop_length = polyline_length(points);
    if loop_length <= 0.0 {
        anyhow::bail!("track loop has zero length");
    }
    let mut remaining = distance_m.rem_euclid(loop_length);
    for (a, b) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        let segment = (b.0 - a.0).hypot(b.1 - a.1);
        if remaining <= segment || segment <= 1e-9 {
            let ratio = if segment <= 1e-9 {
                0.0
            } else {
                remaining / segment
            };
            return Ok((a.0 + (b.0 - a.0) * ratio, a.1 + (b.1 - a.1) * ratio));
        }
        remaining -= segment;
    }
    points
        .last()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("track loop has no points"))
}
