//! Invariant, stage 2 (heuristics §6.3): independent assertion over the final assignments.
//!
//! Deliberately shares no code with the assignment-time filter in `facilities.rs`: it does its
//! own brute-force projection over raw strand points and recomputes everything per assignment.
//! A POI violates the invariant if it is not right of its facility's strand (`d <= 0`), or if its
//! nearest strand of all strands is the opposite direction of the same ref.

use crate::params::{CORRIDOR_VERTEX_WINDOW_KM, EARTH_RADIUS_M, OPPOSITE_DIRECTION_DEG};
use crate::strands::Strand;

/// One final assignment to check.
pub struct Assignment<'a> {
    pub facility: String,
    pub poi: String,
    pub strand: &'a Strand,
    /// Projection km of the facility centroid on `strand` (centre of the +-2 km window).
    pub centroid_km: f64,
    pub poi_lat: f64,
    pub poi_lon: f64,
}

#[derive(Debug, PartialEq)]
pub struct Violation {
    pub facility: String,
    pub poi: String,
    pub reason: String,
}

/// Segment geometry in a local frame around the POI.
struct Seg {
    /// Distance from the POI to the clamped segment, metres.
    dist: f64,
    /// Signed offset (positive = POI right of the direction a -> b).
    side: f64,
    /// Compass bearing a -> b in degrees [0, 360).
    bearing: f64,
}

fn seg_geometry(a: (f64, f64), b: (f64, f64), poi: (f64, f64)) -> Seg {
    let cos0 = poi.0.to_radians().cos();
    let to_xy = |p: (f64, f64)| {
        (
            EARTH_RADIUS_M * (p.1 - poi.1).to_radians() * cos0,
            EARTH_RADIUS_M * (p.0 - poi.0).to_radians(),
        )
    };
    let (ax, ay) = to_xy(a);
    let (bx, by) = to_xy(b);
    let (ex, ey) = (bx - ax, by - ay);
    let len = (ex * ex + ey * ey).sqrt();
    let t = if len == 0.0 {
        0.0
    } else {
        ((-ax * ex - ay * ey) / (len * len)).clamp(0.0, 1.0)
    };
    let (fx, fy) = (ax + t * ex, ay + t * ey);
    // POI is the origin: the POI is right of a -> b when the cross product (b-a) x (poi-a) < 0.
    let cross = ex * (0.0 - ay) - ey * (0.0 - ax);
    Seg {
        dist: (fx * fx + fy * fy).sqrt(),
        side: if len == 0.0 { 0.0 } else { -cross / len },
        bearing: ex.atan2(ey).to_degrees().rem_euclid(360.0),
    }
}

fn bearing_gap(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 {
        360.0 - d
    } else {
        d
    }
}

/// Nearest segment of a strand within the km window (or all segments), brute force.
fn nearest_segment(s: &Strand, poi: (f64, f64), window: Option<(f64, f64)>) -> Option<Seg> {
    let mut best: Option<Seg> = None;
    for i in 0..s.points.len().saturating_sub(1) {
        if let Some((lo, hi)) = window {
            if s.cum_km[i + 1] < lo || s.cum_km[i] > hi {
                continue;
            }
        } else {
            // Cheap rejection of far-away segments (5 km margin) for the all-strands scan.
            let (a, b) = (s.points[i], s.points[i + 1]);
            let m = 0.05;
            if poi.0 < a.lat.min(b.lat) - m
                || poi.0 > a.lat.max(b.lat) + m
                || poi.1 < a.lon.min(b.lon) - 2.0 * m
                || poi.1 > a.lon.max(b.lon) + 2.0 * m
            {
                continue;
            }
        }
        let g = seg_geometry(
            (s.points[i].lat, s.points[i].lon),
            (s.points[i + 1].lat, s.points[i + 1].lon),
            poi,
        );
        if best.as_ref().is_none_or(|b| g.dist < b.dist) {
            best = Some(g);
        }
    }
    best
}

/// Checks all assignments against all strands. Returns every violation.
pub fn check(assignments: &[Assignment<'_>], all_strands: &[Strand]) -> Vec<Violation> {
    let mut out = Vec::new();
    for a in assignments {
        let poi = (a.poi_lat, a.poi_lon);
        let window = (
            a.centroid_km - CORRIDOR_VERTEX_WINDOW_KM,
            a.centroid_km + CORRIDOR_VERTEX_WINDOW_KM,
        );
        let own = nearest_segment(a.strand, poi, Some(window));
        let Some(own) = own else {
            out.push(Violation {
                facility: a.facility.clone(),
                poi: a.poi.clone(),
                reason: "no segment of the assigned strand near the facility".into(),
            });
            continue;
        };
        if own.side <= 0.0 {
            out.push(Violation {
                facility: a.facility.clone(),
                poi: a.poi.clone(),
                reason: format!(
                    "POI is not right of {} (d = {:.1} m)",
                    a.strand.id, own.side
                ),
            });
            continue;
        }
        // Nearest strand over all strands (first minimum wins).
        let mut nearest: Option<(&Strand, Seg)> = None;
        for s in all_strands {
            if let Some(g) = nearest_segment(s, poi, None) {
                if nearest.as_ref().is_none_or(|(_, b)| g.dist < b.dist) {
                    nearest = Some((s, g));
                }
            }
        }
        if let Some((s, g)) = nearest {
            if s.id != a.strand.id
                && s.ref_ == a.strand.ref_
                && bearing_gap(own.bearing, g.bearing) > OPPOSITE_DIRECTION_DEG
            {
                out.push(Violation {
                    facility: a.facility.clone(),
                    poi: a.poi.clone(),
                    reason: format!(
                        "nearest strand {} is the opposite direction of {}",
                        s.id, a.strand.id
                    ),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::local::cumulative_km;
    use crate::geo::LatLon;

    fn strand(id: &str, pts: Vec<LatLon>) -> Strand {
        let cum_km = cumulative_km(&pts);
        Strand {
            id: id.into(),
            ref_: "A 1".into(),
            towards: "N",
            polyline: String::new(),
            length_km: *cum_km.last().unwrap(),
            cum_km,
            points: pts,
            first_way_id: 1,
            way_ids: vec![1],
        }
    }

    fn asg<'a>(s: &'a Strand, lat: f64, lon: f64) -> Assignment<'a> {
        Assignment {
            facility: "osm-way-1".into(),
            poi: "osm-node-9".into(),
            strand: s,
            centroid_km: 2.0,
            poi_lat: lat,
            poi_lon: lon,
        }
    }

    #[test]
    fn detects_left_and_opposite_and_accepts_valid() {
        let north = strand(
            "A1-N-0",
            vec![LatLon::new(50.0, 10.0), LatLon::new(50.05, 10.0)],
        );
        // Southbound strand to the EAST of the northbound one.
        let south = strand(
            "A1-S-0",
            vec![LatLon::new(50.05, 10.0004), LatLon::new(50.0, 10.0004)],
        );
        let all = vec![north.clone(), south];
        // Right of north and nearest to north: fine.
        assert!(check(&[asg(&north, 50.02, 10.0001)], &all).is_empty());
        // Left of north: violation.
        let v = check(&[asg(&north, 50.02, 9.9995)], &all);
        assert_eq!(v.len(), 1);
        assert!(v[0].reason.contains("not right"), "{v:?}");
        // Right of north (d > 0) but nearest to the opposite strand: violation.
        let v = check(&[asg(&north, 50.02, 10.0003)], &all);
        assert_eq!(v.len(), 1);
        assert!(v[0].reason.contains("opposite"), "{v:?}");
    }

    #[test]
    fn other_ref_nearest_is_fine() {
        let north = strand(
            "A1-N-0",
            vec![LatLon::new(50.0, 10.0), LatLon::new(50.05, 10.0)],
        );
        let mut other = strand(
            "A3-S-0",
            vec![LatLon::new(50.05, 10.0004), LatLon::new(50.0, 10.0004)],
        );
        other.ref_ = "A 3".into();
        let all = vec![north.clone(), other];
        assert!(check(&[asg(&north, 50.02, 10.0003)], &all).is_empty());
    }
}
