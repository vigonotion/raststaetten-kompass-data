//! Point in polygon and polygon centroid (local metric coordinates).

use super::local::LocalProj;
use super::LatLon;

/// Ray casting test of `p` against a ring (closed or open) in local coordinates around `p`.
pub fn point_in_polygon(ring: &[LatLon], p: LatLon) -> bool {
    if ring.len() < 3 {
        return false;
    }
    let proj = LocalProj::new(p);
    let xy: Vec<(f64, f64)> = ring.iter().map(|q| proj.to_xy(*q)).collect();
    let mut inside = false;
    let mut j = xy.len() - 1;
    for i in 0..xy.len() {
        let (xi, yi) = xy[i];
        let (xj, yj) = xy[j];
        // Ray from the origin (= p) towards +x.
        if (yi > 0.0) != (yj > 0.0) {
            let x_cross = (xj - xi) * (0.0 - yi) / (yj - yi) + xi;
            if x_cross > 0.0 {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Total area in square metres of all rings (sum of absolute shoelace areas, local coordinates).
pub fn area_m2(rings: &[Vec<LatLon>]) -> f64 {
    let Some(first) = rings.iter().find_map(|r| r.first().copied()) else {
        return 0.0;
    };
    let proj = LocalProj::new(first);
    rings
        .iter()
        .filter(|r| r.len() >= 3)
        .map(|ring| {
            let xy: Vec<(f64, f64)> = ring.iter().map(|q| proj.to_xy(*q)).collect();
            let mut a2 = 0.0;
            for i in 0..xy.len() {
                let (x0, y0) = xy[i];
                let (x1, y1) = xy[(i + 1) % xy.len()];
                a2 += x0 * y1 - x1 * y0;
            }
            a2.abs() / 2.0
        })
        .sum()
}

/// Proper intersection of the segments `a-b` and `c-d` (local metric coordinates).
pub fn segments_intersect_xy(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let orient = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| -> f64 {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let (o1, o2) = (orient(a, b, c), orient(a, b, d));
    let (o3, o4) = (orient(c, d, a), orient(c, d, b));
    (o1 > 0.0) != (o2 > 0.0)
        && (o3 > 0.0) != (o4 > 0.0)
        && o1 != 0.0
        && o2 != 0.0
        && o3 != 0.0
        && o4 != 0.0
}

/// Area-weighted centroid (shoelace) of all outer rings. Falls back to the mean of the
/// vertices when the total area is (almost) zero. Returns `None` for empty input.
pub fn centroid(rings: &[Vec<LatLon>]) -> Option<LatLon> {
    let first = rings.iter().find_map(|r| r.first().copied())?;
    let proj = LocalProj::new(first);
    let mut area_sum = 0.0;
    let mut cx = 0.0;
    let mut cy = 0.0;
    for ring in rings {
        let xy: Vec<(f64, f64)> = ring.iter().map(|q| proj.to_xy(*q)).collect();
        if xy.len() < 3 {
            continue;
        }
        let mut a2 = 0.0; // twice the signed area
        let mut sx = 0.0;
        let mut sy = 0.0;
        for i in 0..xy.len() {
            let (x0, y0) = xy[i];
            let (x1, y1) = xy[(i + 1) % xy.len()];
            let cross = x0 * y1 - x1 * y0;
            a2 += cross;
            sx += (x0 + x1) * cross;
            sy += (y0 + y1) * cross;
        }
        if a2.abs() < 1e-9 {
            continue;
        }
        // centroid of this ring = (sx, sy) / (3 * a2); weight = |area|
        let w = a2.abs() / 2.0;
        cx += sx / (3.0 * a2) * w;
        cy += sy / (3.0 * a2) * w;
        area_sum += w;
    }
    if area_sum > 1e-6 {
        return Some(proj.to_latlon((cx / area_sum, cy / area_sum)));
    }
    // Degenerate: mean of the vertices (ignoring a duplicated closing vertex).
    let mut lat = 0.0;
    let mut lon = 0.0;
    let mut n = 0.0;
    for ring in rings {
        let verts: &[LatLon] = match (ring.first(), ring.last()) {
            (Some(f), Some(l)) if ring.len() > 1 && f == l => &ring[..ring.len() - 1],
            _ => ring,
        };
        for v in verts {
            lat += v.lat;
            lon += v.lon;
            n += 1.0;
        }
    }
    (n > 0.0).then(|| LatLon::new(lat / n, lon / n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<LatLon> {
        vec![
            LatLon::new(50.0, 10.0),
            LatLon::new(50.0, 10.002),
            LatLon::new(50.001, 10.002),
            LatLon::new(50.001, 10.0),
            LatLon::new(50.0, 10.0),
        ]
    }

    #[test]
    fn area_and_intersection() {
        // 0.002 deg lon (~143 m) x 0.001 deg lat (~111 m)
        let a = area_m2(&[square()]);
        assert!((a - 143.0 * 111.2).abs() < 400.0, "{a}");
        assert_eq!(area_m2(&[]), 0.0);
        assert!(segments_intersect_xy(
            (0.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (10.0, 0.0)
        ));
        assert!(!segments_intersect_xy(
            (0.0, 0.0),
            (1.0, 1.0),
            (5.0, 0.0),
            (6.0, 1.0)
        ));
    }

    #[test]
    fn inside_and_outside() {
        let sq = square();
        assert!(point_in_polygon(&sq, LatLon::new(50.0005, 10.001)));
        assert!(!point_in_polygon(&sq, LatLon::new(50.0005, 10.003)));
        assert!(!point_in_polygon(&sq, LatLon::new(50.002, 10.001)));
        assert!(!point_in_polygon(&sq, LatLon::new(49.999, 10.001)));
        assert!(!point_in_polygon(&sq[..2], LatLon::new(50.0, 10.0)));
    }

    #[test]
    fn concave_polygon() {
        // L shape; the notch at the upper right is outside.
        let l = vec![
            LatLon::new(50.000, 10.000),
            LatLon::new(50.000, 10.002),
            LatLon::new(50.001, 10.002),
            LatLon::new(50.001, 10.001),
            LatLon::new(50.002, 10.001),
            LatLon::new(50.002, 10.000),
        ];
        assert!(point_in_polygon(&l, LatLon::new(50.0015, 10.0005)));
        assert!(!point_in_polygon(&l, LatLon::new(50.0015, 10.0015)));
    }

    #[test]
    fn centroid_square_and_weighting() {
        let c = centroid(&[square()]).unwrap();
        assert!(
            (c.lat - 50.0005).abs() < 1e-7 && (c.lon - 10.001).abs() < 1e-7,
            "{c:?}"
        );
        // A second, much smaller ring barely moves the centroid.
        let small = vec![
            LatLon::new(50.01, 10.0),
            LatLon::new(50.01, 10.00001),
            LatLon::new(50.01001, 10.00001),
            LatLon::new(50.01001, 10.0),
        ];
        let c2 = centroid(&[square(), small]).unwrap();
        assert!((c2.lat - 50.0005).abs() < 1e-4);
        // Reverse orientation gives the same result.
        let mut rev = square();
        rev.reverse();
        let c3 = centroid(&[rev]).unwrap();
        assert!((c3.lat - c.lat).abs() < 1e-9 && (c3.lon - c.lon).abs() < 1e-9);
    }

    #[test]
    fn centroid_degenerate_fallback() {
        let line = vec![
            LatLon::new(50.0, 10.0),
            LatLon::new(50.0, 10.002),
            LatLon::new(50.0, 10.004),
        ];
        let c = centroid(&[line]).unwrap();
        assert!((c.lat - 50.0).abs() < 1e-9 && (c.lon - 10.002).abs() < 1e-9);
        assert!(centroid(&[]).is_none());
    }
}
