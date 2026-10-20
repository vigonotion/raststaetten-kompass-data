//! Grid index over strand segments (cells of 0.01 degrees) for candidate and nearest queries.

use crate::geo::distance::bearing_deg;
use crate::geo::local::{project_onto_segments, Projection};
use crate::geo::LatLon;
use crate::strands::Strand;
use std::collections::HashMap;

const CELL_DEG: f64 = 0.01;
/// A cell is at least this wide/high in metres anywhere in Germany (0.01 deg of longitude at 56 N).
const MIN_CELL_M: f64 = 620.0;
/// Largest ring of cells searched for the nearest strand (about 10 km).
const MAX_NEAREST_RING: i32 = 16;

pub type Cell = (i32, i32);

pub fn cell_of(p: LatLon) -> Cell {
    (
        (p.lat / CELL_DEG).floor() as i32,
        (p.lon / CELL_DEG).floor() as i32,
    )
}

/// Projection of a point onto one strand.
#[derive(Clone, Copy, Debug)]
pub struct StrandHit {
    pub strand: usize,
    pub proj: Projection,
}

pub struct StrandIndex<'a> {
    strands: &'a [Strand],
    cells: HashMap<Cell, Vec<(u32, u32)>>,
}

impl<'a> StrandIndex<'a> {
    pub fn new(strands: &'a [Strand]) -> Self {
        let mut cells: HashMap<Cell, Vec<(u32, u32)>> = HashMap::new();
        for (si, s) in strands.iter().enumerate() {
            for (i, w) in s.points.windows(2).enumerate() {
                let (c0, c1) = (cell_of(w[0]), cell_of(w[1]));
                for cy in c0.0.min(c1.0)..=c0.0.max(c1.0) {
                    for cx in c0.1.min(c1.1)..=c0.1.max(c1.1) {
                        cells
                            .entry((cy, cx))
                            .or_default()
                            .push((si as u32, i as u32));
                    }
                }
            }
        }
        StrandIndex { strands, cells }
    }

    /// Segments (strand, segment) in the cells within `ring` cells of `p`'s cell, sorted, unique.
    fn segments_near(&self, p: LatLon, ring: i32) -> Vec<(u32, u32)> {
        let (cy, cx) = cell_of(p);
        let mut out = Vec::new();
        for y in cy - ring..=cy + ring {
            for x in cx - ring..=cx + ring {
                if let Some(v) = self.cells.get(&(y, x)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Per strand the nearest projection among the given sorted segment list.
    fn best_per_strand(&self, segs: &[(u32, u32)], p: LatLon) -> Vec<StrandHit> {
        let mut hits = Vec::new();
        let mut i = 0;
        while i < segs.len() {
            let si = segs[i].0;
            let mut j = i;
            while j < segs.len() && segs[j].0 == si {
                j += 1;
            }
            let s = &self.strands[si as usize];
            if let Some(proj) = project_onto_segments(
                &s.points,
                &s.cum_km,
                segs[i..j].iter().map(|x| x.1 as usize),
                p,
            ) {
                hits.push(StrandHit {
                    strand: si as usize,
                    proj,
                });
            }
            i = j;
        }
        hits
    }

    /// All strands whose nearest segment is at most `radius_m` from `p` (`radius_m` <= 600),
    /// ordered by strand index.
    pub fn within(&self, p: LatLon, radius_m: f64) -> Vec<StrandHit> {
        debug_assert!(radius_m <= MIN_CELL_M);
        let segs = self.segments_near(p, 1);
        let mut hits = self.best_per_strand(&segs, p);
        hits.retain(|h| h.proj.distance_m <= radius_m);
        hits
    }

    /// The nearest strand overall (ties: lower strand index), searching rings of growing size.
    pub fn nearest(&self, p: LatLon) -> Option<StrandHit> {
        let mut ring = 1;
        loop {
            let segs = self.segments_near(p, ring);
            let hits = self.best_per_strand(&segs, p);
            let best = hits
                .into_iter()
                .min_by(|a, b| a.proj.distance_m.total_cmp(&b.proj.distance_m));
            if let Some(b) = best {
                // Anything closer than `ring` cells is guaranteed to lie inside the searched area.
                if b.proj.distance_m <= f64::from(ring) * MIN_CELL_M {
                    return Some(b);
                }
            }
            if ring >= MAX_NEAREST_RING {
                return best;
            }
            ring *= 2;
        }
    }
}

/// Bearing of segment `i` of a strand.
pub fn segment_bearing(s: &Strand, i: usize) -> f64 {
    bearing_deg(s.points[i], s.points[i + 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strand(id: &str, ref_: &str, pts: Vec<LatLon>) -> Strand {
        let cum_km = crate::geo::local::cumulative_km(&pts);
        Strand {
            id: id.into(),
            ref_: ref_.into(),
            towards: "N",
            polyline: String::new(),
            length_km: *cum_km.last().unwrap(),
            cum_km,
            points: pts,
            first_way_id: 1,
            way_ids: vec![1],
        }
    }

    #[test]
    fn within_and_nearest() {
        let north = strand(
            "A1-N-0",
            "A 1",
            vec![LatLon::new(50.0, 10.0), LatLon::new(50.05, 10.0)],
        );
        let south = strand(
            "A1-S-0",
            "A 1",
            vec![LatLon::new(50.05, 10.0005), LatLon::new(50.0, 10.0005)],
        );
        let strands = vec![north, south];
        let idx = StrandIndex::new(&strands);
        let p = LatLon::new(50.02, 10.001); // east of both, ~71 m / ~36 m
        let hits = idx.within(p, 300.0);
        assert_eq!(hits.len(), 2);
        assert!(hits[0].proj.d > 0.0); // right of the northbound strand
        assert!(hits[1].proj.d < 0.0); // left of the southbound strand
        assert_eq!(idx.nearest(p).unwrap().strand, 1);
        assert!(idx.within(LatLon::new(50.02, 10.01), 300.0).is_empty());
        // Far away: found by growing rings.
        let far = idx.nearest(LatLon::new(50.02, 10.08)).unwrap();
        assert!(far.proj.distance_m > 5000.0 && far.proj.distance_m < 7000.0);
        assert!(idx.nearest(LatLon::new(52.0, 12.0)).is_none());
    }
}
