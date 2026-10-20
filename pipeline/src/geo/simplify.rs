//! Douglas-Peucker simplification in local metric coordinates.

use super::local::LocalProj;
use super::LatLon;

fn point_segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (fx, fy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - fx).powi(2) + (p.1 - fy).powi(2)).sqrt()
}

/// Returns the indices of the points kept by Douglas-Peucker with tolerance `tol_m`
/// (perpendicular distance to the segment, metres). Iterative, deterministic.
pub fn douglas_peucker_indices(points: &[LatLon], tol_m: f64) -> Vec<usize> {
    let n = points.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j <= i + 1 {
            continue;
        }
        // The projection origin follows the range so that long lines stay accurate.
        let origin = LatLon::new((points[i].lat + points[j].lat) / 2.0, points[i].lon);
        let proj = LocalProj::new(origin);
        let a = proj.to_xy(points[i]);
        let b = proj.to_xy(points[j]);
        let mut worst = 0.0;
        let mut worst_idx = i;
        for (k, pt) in points.iter().enumerate().take(j).skip(i + 1) {
            let d = point_segment_distance(proj.to_xy(*pt), a, b);
            if d > worst {
                worst = d;
                worst_idx = k;
            }
        }
        if worst > tol_m {
            keep[worst_idx] = true;
            stack.push((i, worst_idx));
            stack.push((worst_idx, j));
        }
    }
    keep.iter()
        .enumerate()
        .filter_map(|(i, k)| k.then_some(i))
        .collect()
}

pub fn douglas_peucker(points: &[LatLon], tol_m: f64) -> Vec<LatLon> {
    douglas_peucker_indices(points, tol_m)
        .into_iter()
        .map(|i| points[i])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_points_within_tolerance() {
        // 0.0001 deg lon at lat 50 = 7.2 m
        let pts = [
            LatLon::new(50.000, 10.0),
            LatLon::new(50.005, 10.0001),
            LatLon::new(50.010, 10.0),
        ];
        assert_eq!(douglas_peucker(&pts, 15.0).len(), 2);
    }

    #[test]
    fn keeps_points_beyond_tolerance() {
        // 0.0003 deg lon = 21.5 m
        let pts = [
            LatLon::new(50.000, 10.0),
            LatLon::new(50.005, 10.0003),
            LatLon::new(50.010, 10.0),
        ];
        assert_eq!(douglas_peucker(&pts, 15.0).len(), 3);
    }

    #[test]
    fn keeps_endpoints_and_handles_short_input() {
        assert!(douglas_peucker(&[], 15.0).is_empty());
        let one = [LatLon::new(1.0, 1.0)];
        assert_eq!(douglas_peucker(&one, 15.0), one.to_vec());
        let pts: Vec<LatLon> = (0..100)
            .map(|i| LatLon::new(50.0 + i as f64 * 1e-4, 10.0))
            .collect();
        let s = douglas_peucker(&pts, 15.0);
        assert_eq!(s, vec![pts[0], pts[99]]);
    }

    #[test]
    fn recursion_keeps_both_corners() {
        let pts = [
            LatLon::new(50.00, 10.00),
            LatLon::new(50.01, 10.00),
            LatLon::new(50.01, 10.01),
            LatLon::new(50.02, 10.01),
        ];
        assert_eq!(douglas_peucker(&pts, 15.0).len(), 4);
    }
}
