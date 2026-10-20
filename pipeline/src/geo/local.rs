//! Local equirectangular projection, signed lateral offset, projection onto a polyline.

use super::distance::haversine_m;
use super::LatLon;
use crate::params::EARTH_RADIUS_M;

/// Equirectangular projection around a reference point: `x = R·Δλ·cos(φ0)`, `y = R·Δφ`.
#[derive(Clone, Copy, Debug)]
pub struct LocalProj {
    origin: LatLon,
    cos_lat0: f64,
}

impl LocalProj {
    pub fn new(origin: LatLon) -> Self {
        LocalProj {
            origin,
            cos_lat0: origin.lat.to_radians().cos(),
        }
    }

    pub fn to_xy(self, p: LatLon) -> (f64, f64) {
        let x = EARTH_RADIUS_M * (p.lon - self.origin.lon).to_radians() * self.cos_lat0;
        let y = EARTH_RADIUS_M * (p.lat - self.origin.lat).to_radians();
        (x, y)
    }

    pub fn to_latlon(self, xy: (f64, f64)) -> LatLon {
        LatLon::new(
            self.origin.lat + (xy.1 / EARTH_RADIUS_M).to_degrees(),
            self.origin.lon + (xy.0 / (EARTH_RADIUS_M * self.cos_lat0)).to_degrees(),
        )
    }
}

/// Signed lateral offset of `p` relative to the directed segment `a -> b`
/// (all in local metric coordinates). `d > 0` = right of the direction of travel.
/// Schema §3: `cross = (B−A) × (P−A)`, `d = −cross / |B−A|`.
pub fn signed_offset_xy(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    if len == 0.0 {
        return 0.0;
    }
    let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    -cross / len
}

/// Cumulative Haversine distance in km from point 0 to every point.
pub fn cumulative_km(points: &[LatLon]) -> Vec<f64> {
    let mut cum = Vec::with_capacity(points.len());
    let mut acc = 0.0;
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            acc += haversine_m(points[i - 1], *p) / 1000.0;
        }
        cum.push(acc);
    }
    cum
}

/// Result of projecting a point onto a polyline.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    /// Index of the nearest segment (`points[segment] -> points[segment + 1]`).
    pub segment: usize,
    /// Position along the polyline in km.
    pub km: f64,
    /// Signed lateral offset in metres to the nearest segment (`> 0` = right).
    pub d: f64,
    /// Distance in metres from the point to the (clamped) foot point.
    pub distance_m: f64,
    #[allow(dead_code)]
    pub foot: LatLon,
}

/// Projects `p` onto the nearest segment of the polyline `points` (`cum` = `cumulative_km`).
/// Returns `None` for fewer than two points.
pub fn project_onto_polyline(points: &[LatLon], cum: &[f64], p: LatLon) -> Option<Projection> {
    if points.len() < 2 {
        return None;
    }
    project_onto_segments(points, cum, 0..points.len() - 1, p)
}

/// Like `project_onto_polyline`, but only over the given segment indices (`points[i] -> points[i+1]`).
/// Ties are resolved in favour of the earlier entry. Returns `None` for an empty selection.
pub fn project_onto_segments(
    points: &[LatLon],
    cum: &[f64],
    segments: impl IntoIterator<Item = usize>,
    p: LatLon,
) -> Option<Projection> {
    let proj = LocalProj::new(p);
    let origin = (0.0, 0.0);
    let mut best: Option<Projection> = None;
    for i in segments {
        if i + 1 >= points.len() {
            continue;
        }
        let a_xy = proj.to_xy(points[i]);
        let b_xy = proj.to_xy(points[i + 1]);
        let (dx, dy) = (b_xy.0 - a_xy.0, b_xy.1 - a_xy.1);
        let len2 = dx * dx + dy * dy;
        let t = if len2 == 0.0 {
            0.0
        } else {
            (((origin.0 - a_xy.0) * dx + (origin.1 - a_xy.1) * dy) / len2).clamp(0.0, 1.0)
        };
        let fx = a_xy.0 + t * dx;
        let fy = a_xy.1 + t * dy;
        let dist = (fx * fx + fy * fy).sqrt();
        if best.is_none_or(|b| dist < b.distance_m) {
            let (a, b) = (points[i], points[i + 1]);
            let foot = if t <= 0.0 {
                a
            } else if t >= 1.0 {
                b
            } else {
                LatLon::new(a.lat + t * (b.lat - a.lat), a.lon + t * (b.lon - a.lon))
            };
            best = Some(Projection {
                segment: i,
                km: cum[i] + haversine_m(a, foot) / 1000.0,
                d: signed_offset_xy(a_xy, b_xy, origin),
                distance_m: dist,
                foot,
            });
        }
    }
    best
}

/// Range of segment indices whose km span intersects `[lo_km, hi_km]`.
/// Always non-empty for polylines with at least two points.
pub fn segments_in_window(cum: &[f64], lo_km: f64, hi_km: f64) -> std::ops::Range<usize> {
    let n_seg = cum.len().saturating_sub(1);
    if n_seg == 0 {
        return 0..0;
    }
    let first = cum
        .partition_point(|&c| c < lo_km)
        .saturating_sub(1)
        .min(n_seg - 1);
    let last_excl = cum.partition_point(|&c| c <= hi_km).clamp(first + 1, n_seg);
    first..last_excl
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_projection_scale() {
        let proj = LocalProj::new(LatLon::new(50.0, 10.0));
        let (x, y) = proj.to_xy(LatLon::new(50.0, 10.001));
        assert!((x - 71.47).abs() < 0.05, "{x}"); // 111.195 m per 0.001 deg * cos(50 deg)
        assert!(y.abs() < 1e-9);
        let (x, y) = proj.to_xy(LatLon::new(50.001, 10.0));
        assert!(x.abs() < 1e-9);
        assert!((y - 111.195).abs() < 0.001);
        let back = proj.to_latlon(proj.to_xy(LatLon::new(50.002, 10.003)));
        assert!((back.lat - 50.002).abs() < 1e-12 && (back.lon - 10.003).abs() < 1e-12);
    }

    #[test]
    fn offset_sign_convention() {
        // Northbound segment, point to the east = right => d > 0.
        let d = signed_offset_xy((0.0, 0.0), (0.0, 100.0), (10.0, 50.0));
        assert!((d - 10.0).abs() < 1e-9);
        let d = signed_offset_xy((0.0, 0.0), (0.0, 100.0), (-7.0, 50.0));
        assert!((d + 7.0).abs() < 1e-9);
        // Southbound: east is left.
        let d = signed_offset_xy((0.0, 100.0), (0.0, 0.0), (10.0, 50.0));
        assert!((d + 10.0).abs() < 1e-9);
    }

    fn north_line() -> Vec<LatLon> {
        vec![
            LatLon::new(50.0, 10.0),
            LatLon::new(50.01, 10.0),
            LatLon::new(50.02, 10.0),
        ]
    }

    #[test]
    fn projection_east_of_northbound_is_right() {
        let pts = north_line();
        let cum = cumulative_km(&pts);
        let p = LatLon::new(50.005, 10.001);
        let pr = project_onto_polyline(&pts, &cum, p).unwrap();
        assert_eq!(pr.segment, 0);
        assert!(pr.d > 0.0, "{}", pr.d);
        assert!((pr.d - 71.5).abs() < 0.5, "{}", pr.d);
        assert!((pr.distance_m - pr.d.abs()).abs() < 1e-6);
        assert!((pr.km - 0.5559).abs() < 0.001, "{}", pr.km);
        let west = project_onto_polyline(&pts, &cum, LatLon::new(50.005, 9.999)).unwrap();
        assert!(west.d < 0.0);
    }

    #[test]
    fn projection_second_segment_and_clamping() {
        let pts = north_line();
        let cum = cumulative_km(&pts);
        let pr = project_onto_polyline(&pts, &cum, LatLon::new(50.015, 10.0)).unwrap();
        assert_eq!(pr.segment, 1);
        assert!((pr.km - cum[1] - 0.5559).abs() < 0.001);
        // Beyond the end: clamped to last point, km = total length.
        let end = project_onto_polyline(&pts, &cum, LatLon::new(50.03, 10.0)).unwrap();
        assert_eq!(end.km, cum[2]);
        assert!((end.distance_m - 1111.95).abs() < 0.5);
        // Before the start.
        let start = project_onto_polyline(&pts, &cum, LatLon::new(49.99, 10.0)).unwrap();
        assert_eq!(start.km, 0.0);
    }

    #[test]
    fn window_selection() {
        let pts = north_line();
        let cum = cumulative_km(&pts); // 0, 1.112, 2.224
        assert_eq!(segments_in_window(&cum, 0.0, 10.0), 0..2);
        assert_eq!(segments_in_window(&cum, 1.5, 1.6), 1..2);
        assert_eq!(segments_in_window(&cum, 0.2, 0.3), 0..1);
        assert_eq!(segments_in_window(&cum, 5.0, 6.0), 1..2);
        let pr = project_onto_segments(&pts, &cum, 1..2, LatLon::new(50.005, 10.001)).unwrap();
        assert_eq!(pr.segment, 1); // forced onto the second segment (clamped to its start)
    }

    #[test]
    fn projection_needs_two_points() {
        assert!(
            project_onto_polyline(&[LatLon::new(1.0, 1.0)], &[0.0], LatLon::new(1.0, 1.0))
                .is_none()
        );
    }

    #[test]
    fn cumulative() {
        let cum = cumulative_km(&north_line());
        assert_eq!(cum[0], 0.0);
        assert!((cum[2] - 2.2239).abs() < 0.001);
    }
}
