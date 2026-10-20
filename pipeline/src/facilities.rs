//! Rastanlagen: strand assignment (heuristics §4), POI corridor and invariant (§6),
//! brands and amenities (§5, §6.4).

use crate::amenities;
use crate::brands::{Brand, Matcher};
use crate::geo::distance::angle_diff_deg;
use crate::geo::local::LocalProj;
use crate::geo::local::{project_onto_segments, segments_in_window, Projection};
use crate::geo::polygon::{area_m2, centroid, point_in_polygon, segments_intersect_xy};
use crate::geo::{round_to, LatLon};
use crate::index::{cell_of, segment_bearing, Cell, StrandHit, StrandIndex};
use crate::osm::model::{Model, Tags};
use crate::params::*;
use crate::strands::Strand;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjType {
    Node,
    Way,
    Relation,
}

impl ObjType {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjType::Node => "node",
            ObjType::Way => "way",
            ObjType::Relation => "relation",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjRef {
    pub typ: ObjType,
    pub id: i64,
}

impl ObjRef {
    pub fn base_id(&self) -> String {
        format!("osm-{}-{}", self.typ.as_str(), self.id)
    }
    pub fn url(&self) -> String {
        format!(
            "https://www.openstreetmap.org/{}/{}",
            self.typ.as_str(),
            self.id
        )
    }
}

/// A `highway=services` object with its geometry.
pub struct Source {
    pub obj: ObjRef,
    pub tags: Tags,
    /// Outer rings (empty for node facilities).
    pub rings: Vec<Vec<LatLon>>,
    pub centroid: LatLon,
}

/// A POI (node, or closed way located by its centroid).
pub struct Poi<'a> {
    pub obj: ObjRef,
    pub pos: LatLon,
    pub tags: &'a Tags,
}

// ---------------------------------------------------------------------------------------------
// Collecting objects from the model
// ---------------------------------------------------------------------------------------------

/// Joins way node lists end to end into rings. Closed ways are rings by themselves; open ways
/// are chained by shared end nodes (reversing as needed). Unclosed leftovers stay as they are.
pub fn assemble_rings(ways: &[Vec<i64>]) -> Vec<Vec<i64>> {
    let mut remaining: Vec<Vec<i64>> = ways.iter().filter(|w| w.len() >= 2).cloned().collect();
    let mut rings = Vec::new();
    while !remaining.is_empty() {
        let mut ring = remaining.remove(0);
        while ring.first() != ring.last() {
            let end = *ring.last().expect("non-empty ring");
            let Some(pos) = remaining
                .iter()
                .position(|w| w.first() == Some(&end) || w.last() == Some(&end))
            else {
                break;
            };
            let mut next = remaining.remove(pos);
            if next.first() != Some(&end) {
                next.reverse();
            }
            ring.extend_from_slice(&next[1..]);
        }
        rings.push(ring);
    }
    rings
}

fn ring_coords(nodes: &[i64], model: &Model) -> Vec<LatLon> {
    let mut pts: Vec<LatLon> = nodes
        .iter()
        .filter_map(|n| model.coords.get(n).copied())
        .collect();
    pts.dedup();
    pts
}

/// Facility sources sorted by (type, id). Also returns the number of objects skipped because
/// their geometry is incomplete (nodes outside the extract).
pub fn collect_sources(model: &Model) -> (Vec<Source>, usize) {
    let mut out: Vec<Source> = Vec::new();
    let mut skipped = 0usize;
    for (id, n) in &model.service_nodes {
        out.push(Source {
            obj: ObjRef {
                typ: ObjType::Node,
                id: *id,
            },
            tags: n.tags.clone(),
            rings: Vec::new(),
            centroid: n.pos,
        });
    }
    let relation_members: HashSet<i64> = model
        .service_relations
        .values()
        .flat_map(|r| r.outer_ways.iter().copied())
        .collect();
    for (id, w) in &model.service_ways {
        if relation_members.contains(id) {
            continue; // part of a multipolygon facility
        }
        let ring = ring_coords(&w.nodes, model);
        match (ring.len() >= 3).then(|| centroid(std::slice::from_ref(&ring))) {
            Some(Some(c)) => out.push(Source {
                obj: ObjRef {
                    typ: ObjType::Way,
                    id: *id,
                },
                tags: w.tags.clone(),
                rings: vec![ring],
                centroid: c,
            }),
            _ => skipped += 1,
        }
    }
    for (id, r) in &model.service_relations {
        let lists: Vec<Vec<i64>> = r
            .outer_ways
            .iter()
            .filter_map(|w| model.member_ways.get(w).cloned())
            .collect();
        let rings: Vec<Vec<LatLon>> = assemble_rings(&lists)
            .iter()
            .map(|r| ring_coords(r, model))
            .filter(|r| r.len() >= 3)
            .collect();
        match centroid(&rings) {
            Some(c) if !rings.is_empty() => out.push(Source {
                obj: ObjRef {
                    typ: ObjType::Relation,
                    id: *id,
                },
                tags: r.tags.clone(),
                rings,
                centroid: c,
            }),
            _ => skipped += 1,
        }
    }
    out.sort_by_key(|s| s.obj);
    (out, skipped)
}

/// POIs sorted by (type, id), without the facility objects themselves.
pub fn collect_pois<'a>(model: &'a Model, exclude: &HashSet<ObjRef>) -> Vec<Poi<'a>> {
    let mut out: Vec<Poi<'a>> = Vec::new();
    for (id, n) in &model.poi_nodes {
        let obj = ObjRef {
            typ: ObjType::Node,
            id: *id,
        };
        if !exclude.contains(&obj) {
            out.push(Poi {
                obj,
                pos: n.pos,
                tags: &n.tags,
            });
        }
    }
    for (id, w) in &model.poi_ways {
        let obj = ObjRef {
            typ: ObjType::Way,
            id: *id,
        };
        if exclude.contains(&obj) {
            continue;
        }
        let ring = ring_coords(&w.nodes, model);
        if ring.len() < 3 {
            continue;
        }
        if let Some(c) = centroid(std::slice::from_ref(&ring)) {
            out.push(Poi {
                obj,
                pos: c,
                tags: &w.tags,
            });
        }
    }
    out.sort_by_key(|p| p.obj);
    out
}

/// Positions of all truck parkings (nodes, and centroids of closed ways), sorted by object ID
/// (heuristics §7, D-32).
pub fn collect_truck_parkings(model: &Model) -> Vec<LatLon> {
    let mut out: Vec<LatLon> = model.truck_parking_nodes.values().copied().collect();
    for nodes in model.truck_parking_ways.values() {
        let ring = ring_coords(nodes, model);
        if ring.len() < 3 {
            continue;
        }
        if let Some(c) = centroid(std::slice::from_ref(&ring)) {
            out.push(c);
        }
    }
    out
}

pub struct PoiGrid {
    cells: HashMap<Cell, Vec<u32>>,
}

impl PoiGrid {
    pub fn new(pois: &[Poi<'_>]) -> Self {
        let mut cells: HashMap<Cell, Vec<u32>> = HashMap::new();
        for (i, p) in pois.iter().enumerate() {
            cells.entry(cell_of(p.pos)).or_default().push(i as u32);
        }
        PoiGrid { cells }
    }

    /// Indices of POIs in all cells touched by the circle of `radius_m` around `p`, sorted.
    pub fn near(&self, p: LatLon, radius_m: f64) -> Vec<u32> {
        let dlat = radius_m / 110_000.0;
        let dlon = radius_m / (110_000.0 * p.lat.to_radians().cos().max(0.1));
        let (y0, x0) = cell_of(LatLon::new(p.lat - dlat, p.lon - dlon));
        let (y1, x1) = cell_of(LatLon::new(p.lat + dlat, p.lon + dlon));
        let mut out = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                if let Some(v) = self.cells.get(&(y, x)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Exclusion and merging before assignment (heuristics §4.1a)
// ---------------------------------------------------------------------------------------------

/// A services object kept out of the Rastanlage pass (Autohof-like, deferred to phase 4).
pub struct Deferred {
    pub obj: ObjRef,
    pub name: String,
    pub reason: String,
    /// Chosen strand and signed d, if there is one.
    pub strand: Option<(usize, f64)>,
    /// The services object itself (input for the Autohof step, D-31).
    pub source: Source,
}

/// Duplicates merged into one kept object.
pub struct MergeGroup {
    pub kept: ObjRef,
    pub name: String,
    pub strand: usize,
    pub km: f64,
    pub dropped: Vec<(ObjRef, String)>,
}

pub struct Prepared {
    pub sources: Vec<Source>,
    pub deferred: Vec<Deferred>,
    pub merged: Vec<MergeGroup>,
}

fn display_name(tags: &Tags) -> String {
    tags.get("name").unwrap_or("-").to_string()
}

/// `Autohof`, `Truck` or `Maxi` (case-insensitive) in `name`, `operator` or `brand`.
pub fn autohof_like_reason(tags: &Tags) -> Option<String> {
    for key in ["name", "operator", "brand"] {
        if let Some(v) = tags.get(key) {
            let lower = v.to_lowercase();
            if let Some(k) = AUTOHOF_LIKE_KEYWORDS.iter().find(|k| lower.contains(**k)) {
                return Some(format!("{key} contains \"{k}\""));
            }
        }
    }
    None
}

fn inside_other(a: &Source, b: &Source) -> bool {
    b.rings.iter().any(|r| point_in_polygon(r, a.centroid))
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// Excludes Autohof-like objects and merges duplicates (heuristics §4.1a). `sources` must be
/// sorted by object; the result keeps that order.
pub fn prepare(sources: Vec<Source>, strands: &[Strand], index: &StrandIndex<'_>) -> Prepared {
    let n = sources.len();
    let chosen: Vec<Option<StrandHit>> = sources
        .iter()
        .map(|s| {
            let hits = index.within(s.centroid, CANDIDATE_MAX_DISTANCE_M);
            choose_strand(&hits, strands).map(|(c, _)| c)
        })
        .collect();
    // Merge first (D-33): same strand, km within 0.35, centroid of one inside the polygon of the
    // other. The Autohof-like exclusion is judged on the merged group afterwards.
    let mut cand: Vec<(usize, f64, usize)> = (0..n)
        .filter_map(|i| chosen[i].map(|c| (c.strand, c.proj.km, i)))
        .collect();
    cand.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut parent: Vec<usize> = (0..n).collect();
    for x in 0..cand.len() {
        for y in x + 1..cand.len() {
            if cand[y].0 != cand[x].0 || cand[y].1 - cand[x].1 > MERGE_MAX_KM_DIFF {
                break;
            }
            let (a, b) = (&sources[cand[x].2], &sources[cand[y].2]);
            if inside_other(a, b) || inside_other(b, a) {
                let (ra, rb) = (find(&mut parent, cand[x].2), find(&mut parent, cand[y].2));
                if ra != rb {
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(_, _, i) in &cand {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut deferred_idx: Vec<(usize, String)> = Vec::new();
    let mut alive: Vec<bool> = vec![true; n];
    let mut slots: Vec<Option<Source>> = sources.into_iter().map(Some).collect();
    // Objects without a strand within 300 m: only the keyword test applies.
    for i in 0..n {
        if chosen[i].is_none() {
            let s = slots[i].as_ref().expect("present");
            if let Some(reason) = autohof_like_reason(&s.tags) {
                alive[i] = false;
                deferred_idx.push((i, reason));
            }
        }
    }
    let mut merged = Vec::new();
    for mut members in groups.into_values() {
        members.sort_unstable(); // input order = (type, id)
        let like = members
            .iter()
            .find_map(|&m| autohof_like_reason(&slots[m].as_ref().expect("present").tags));
        let area = |i: usize| area_m2(&slots[i].as_ref().expect("present").rings);
        let has_poly = |i: usize| !slots[i].as_ref().expect("present").rings.is_empty();
        let kept = *members
            .iter()
            .max_by(|&&a, &&b| {
                has_poly(a)
                    .cmp(&has_poly(b))
                    .then(area(a).total_cmp(&area(b)))
                    .then(b.cmp(&a)) // smaller (type, id) wins
            })
            .expect("non-empty group");
        let c = chosen[kept].expect("candidates have a strand");
        if members.len() > 1 {
            let mut fills: Vec<(&str, String)> = Vec::new();
            let mut dropped = Vec::new();
            for &m in members.iter().filter(|&&m| m != kept) {
                let d = slots[m].take().expect("present");
                alive[m] = false;
                for key in ["name", "brand", "shower", "opening_hours"] {
                    if let Some(v) = d.tags.get(key) {
                        if !fills.iter().any(|(k, _)| *k == key) {
                            fills.push((key, v.to_string()));
                        }
                    }
                }
                dropped.push((d.obj, display_name(&d.tags)));
            }
            let k = slots[kept].as_mut().expect("present");
            for (key, v) in fills {
                if k.tags.get(key).is_none() {
                    k.tags.0.push((key.to_string(), v));
                }
            }
            merged.push(MergeGroup {
                kept: k.obj,
                name: display_name(&k.tags),
                strand: c.strand,
                km: c.proj.km,
                dropped,
            });
        }
        let far = (c.proj.d.abs() > RASTANLAGE_MAX_DISTANCE_M).then(|| {
            format!(
                "|d| = {:.0} m > {:.0} m",
                c.proj.d.abs(),
                RASTANLAGE_MAX_DISTANCE_M
            )
        });
        if let Some(reason) = like.or(far) {
            alive[kept] = false;
            deferred_idx.push((kept, reason));
        }
    }
    merged.sort_by_key(|m| m.kept);
    let mut deferred: Vec<Deferred> = deferred_idx
        .into_iter()
        .map(|(i, reason)| {
            let source = slots[i].take().expect("deferred source present");
            Deferred {
                obj: source.obj,
                name: display_name(&source.tags),
                reason,
                strand: chosen[i].map(|c| (c.strand, c.proj.d)),
                source,
            }
        })
        .collect();
    deferred.sort_by_key(|d| d.obj);
    let sources: Vec<Source> = slots
        .into_iter()
        .zip(&alive)
        .filter_map(|(s, &a)| if a { s } else { None })
        .collect();
    Prepared {
        sources,
        deferred,
        merged,
    }
}

// ---------------------------------------------------------------------------------------------
// Assignment
// ---------------------------------------------------------------------------------------------

/// A facility placed on its strand(s) with assigned POIs and derived attributes.
pub struct Placed {
    pub src: usize,
    pub strand: usize,
    pub proj: Projection,
    /// Further strands of other refs on shared sections: (strand index, km).
    pub extra: Vec<(usize, f64)>,
    pub pois: Vec<usize>,
    pub brands: BTreeSet<usize>,
    pub shower: bool,
    pub fast_charger: bool,
    pub open24h: bool,
}

pub struct Unassigned {
    pub src: usize,
    /// Nearest strand: (strand index, distance in m, signed d in m).
    pub nearest: Option<(usize, f64, f64)>,
}

pub struct BothSides {
    pub src: usize,
    pub assigned: usize,
    /// Which §4.3 rule fired.
    pub reason: &'static str,
    /// Own strand and the opposite strand: (strand index, signed d).
    pub strands: Vec<(usize, f64)>,
}

pub struct ExcludedPoi {
    pub src: usize,
    pub poi: usize,
    pub d: f64,
    pub reason: &'static str,
}

pub struct MultiClaim {
    pub poi: usize,
    pub winner: usize,
    pub others: Vec<usize>,
}

#[derive(Default)]
pub struct Assignment {
    pub placed: Vec<Placed>,
    pub unassigned: Vec<Unassigned>,
    pub both_sides: Vec<BothSides>,
    pub excluded: Vec<ExcludedPoi>,
    pub multi_claims: Vec<MultiClaim>,
    /// Unmatched `brand` values of facilities and assigned POIs: occurrences and the distinct
    /// facilities (indices into `placed`) they appear at.
    pub unmatched_brands: BTreeMap<String, (usize, BTreeSet<usize>)>,
}

/// Opposite direction (heuristics §3.3) from the local bearings of two strands.
fn is_opposite(a: &Strand, seg_a: usize, b: &Strand, seg_b: usize) -> bool {
    a.ref_ == b.ref_
        && a.id != b.id
        && angle_diff_deg(segment_bearing(a, seg_a), segment_bearing(b, seg_b))
            > OPPOSITE_DIRECTION_DEG
}

/// Corridor of one facility relative to its assigned strand (heuristics §6.1, §6.2).
enum Corridor {
    Polygon {
        s_min: f64,
        s_max: f64,
        d_min: f64,
        d_max: f64,
    },
    Node {
        s: f64,
        d: f64,
    },
}

impl Corridor {
    /// Search radius in metres around the centroid that covers the whole corridor.
    fn radius_m(&self, s_c: f64, d_c: f64) -> f64 {
        let (ds, dd) = match *self {
            Corridor::Polygon {
                s_min,
                s_max,
                d_min,
                d_max,
            } => (
                (s_min - s_c).abs().max((s_max - s_c).abs()) * 1000.0,
                (d_min - d_c)
                    .abs()
                    .max((d_max + CORRIDOR_OUTER_BUFFER_M - d_c).abs()),
            ),
            Corridor::Node { s, d } => (
                NODE_CORRIDOR_HALF_LENGTH_KM * 1000.0 + (s - s_c).abs() * 1000.0,
                (d - NODE_CORRIDOR_INNER_M - d_c)
                    .abs()
                    .max((d + NODE_CORRIDOR_OUTER_M - d_c).abs()),
            ),
        };
        (ds * ds + dd * dd).sqrt() * 1.1 + 60.0
    }

    fn contains(&self, s: f64, d: f64) -> bool {
        match *self {
            Corridor::Polygon {
                s_min,
                s_max,
                d_min,
                d_max,
            } => s >= s_min && s <= s_max && d >= d_min && d <= d_max + CORRIDOR_OUTER_BUFFER_M,
            Corridor::Node { s: sn, d: dn } => {
                (s - sn).abs() <= NODE_CORRIDOR_HALF_LENGTH_KM
                    && d >= (dn - NODE_CORRIDOR_INNER_M).max(0.0)
                    && d <= dn + NODE_CORRIDOR_OUTER_M
            }
        }
    }
}

struct Claim {
    poi: usize,
    fac: usize,
    score: f64,
}

pub struct Context<'a> {
    pub strands: &'a [Strand],
    pub index: &'a StrandIndex<'a>,
    pub matcher: &'a Matcher,
    pub brands: &'a [Brand],
}

/// Chooses the strand(s) of a facility (heuristics §4.2–4.4).
fn choose_strand(
    hits: &[StrandHit],
    strands: &[Strand],
) -> Option<(StrandHit, Vec<(StrandHit, f64)>)> {
    let mut valid: Vec<&StrandHit> = hits.iter().filter(|h| h.proj.d > 0.0).collect();
    valid.sort_by(|a, b| {
        a.proj
            .d
            .abs()
            .total_cmp(&b.proj.d.abs())
            .then_with(|| strands[a.strand].id.cmp(&strands[b.strand].id))
    });
    let chosen = **valid.first()?;
    // Shared sections: one more strand per other ref within 5 m of |d|.
    let mut extra: Vec<(StrandHit, f64)> = Vec::new();
    let mut seen_refs: BTreeSet<&str> = BTreeSet::new();
    seen_refs.insert(strands[chosen.strand].ref_.as_str());
    for h in valid.iter().skip(1) {
        let r = strands[h.strand].ref_.as_str();
        if seen_refs.contains(r) {
            continue;
        }
        if (h.proj.d.abs() - chosen.proj.d.abs()).abs() <= SHARED_SECTION_TOLERANCE_M {
            seen_refs.insert(r);
            extra.push((**h, h.proj.km));
        }
    }
    Some((chosen, extra))
}

/// Facilities on both sides (§4.3, P-03). Relative to an opposite strand within 100 m of the
/// centroid: the polygon crosses it, a polygon vertex reaches more than 15 m beyond the own
/// carriageway (`d_own < -15 m`), or the centroid is nearer to it than to the own strand.
/// Node facilities: only the last rule.
fn both_sides(
    si: usize,
    src: &Source,
    chosen: &StrandHit,
    hits: &[StrandHit],
    strands: &[Strand],
) -> Option<BothSides> {
    let own = &strands[chosen.strand];
    for h in hits {
        if h.strand == chosen.strand
            || h.proj.distance_m > BOTH_SIDES_DISTANCE_M
            || !is_opposite(own, chosen.proj.segment, &strands[h.strand], h.proj.segment)
        {
            continue;
        }
        let other = &strands[h.strand];
        let mut reason = None;
        if !src.rings.is_empty() {
            let lp = LocalProj::new(src.centroid);
            let win = segments_in_window(&other.cum_km, h.proj.km - 1.0, h.proj.km + 1.0);
            let crosses = src.rings.iter().any(|ring| {
                ring.windows(2).any(|e| {
                    let (a, b) = (lp.to_xy(e[0]), lp.to_xy(e[1]));
                    win.clone().any(|i| {
                        i + 1 < other.points.len()
                            && segments_intersect_xy(
                                a,
                                b,
                                lp.to_xy(other.points[i]),
                                lp.to_xy(other.points[i + 1]),
                            )
                    })
                })
            });
            if crosses {
                reason = Some("polygon crosses the opposite strand");
            } else {
                let own_win = segments_in_window(
                    &own.cum_km,
                    chosen.proj.km - CORRIDOR_VERTEX_WINDOW_KM,
                    chosen.proj.km + CORRIDOR_VERTEX_WINDOW_KM,
                );
                let beyond = src.rings.iter().flatten().any(|v| {
                    project_onto_segments(&own.points, &own.cum_km, own_win.clone(), *v)
                        .is_some_and(|p| p.d < -BOTH_SIDES_VERTEX_LEFT_M)
                });
                if beyond {
                    reason = Some("polygon vertex beyond the own carriageway (d < -15 m)");
                }
            }
        }
        if reason.is_none() && h.proj.distance_m < chosen.proj.distance_m {
            reason = Some("centroid nearer to the opposite strand");
        }
        if let Some(reason) = reason {
            return Some(BothSides {
                src: si,
                assigned: chosen.strand,
                reason,
                strands: vec![(chosen.strand, chosen.proj.d), (h.strand, h.proj.d)],
            });
        }
    }
    None
}

/// Runs the whole assignment: strands, corridor, invariant filter, brands and amenities.
pub fn assign(sources: &[Source], pois: &[Poi<'_>], ctx: &Context<'_>) -> Assignment {
    let strands = ctx.strands;
    let grid = PoiGrid::new(pois);
    let mut result = Assignment::default();
    let mut claims: Vec<Claim> = Vec::new();

    for (si, src) in sources.iter().enumerate() {
        let hits = ctx.index.within(src.centroid, CANDIDATE_MAX_DISTANCE_M);
        let Some((chosen, extra)) = choose_strand(&hits, strands) else {
            let nearest = ctx
                .index
                .nearest(src.centroid)
                .map(|h| (h.strand, h.proj.distance_m, h.proj.d));
            result.unassigned.push(Unassigned { src: si, nearest });
            continue;
        };
        if let Some(b) = both_sides(si, src, &chosen, &hits, strands) {
            result.both_sides.push(b);
        }
        let strand = &strands[chosen.strand];
        let pl = result.placed.len();
        result.placed.push(Placed {
            src: si,
            strand: chosen.strand,
            proj: chosen.proj,
            extra: extra.iter().map(|(h, km)| (h.strand, *km)).collect(),
            pois: Vec::new(),
            brands: BTreeSet::new(),
            shower: false,
            fast_charger: false,
            open24h: false,
        });

        // Corridor (§6.1, §6.2).
        let (km_c, d_c) = (chosen.proj.km, chosen.proj.d);
        let window = segments_in_window(
            &strand.cum_km,
            km_c - CORRIDOR_VERTEX_WINDOW_KM,
            km_c + CORRIDOR_VERTEX_WINDOW_KM,
        );
        let project =
            |p: LatLon| project_onto_segments(&strand.points, &strand.cum_km, window.clone(), p);
        let corridor = if src.rings.is_empty() {
            Corridor::Node { s: km_c, d: d_c }
        } else {
            let (mut s_min, mut s_max) = (f64::INFINITY, f64::NEG_INFINITY);
            let (mut d_min, mut d_max) = (f64::INFINITY, f64::NEG_INFINITY);
            for v in src.rings.iter().flatten() {
                if let Some(pr) = project(*v) {
                    s_min = s_min.min(pr.km);
                    s_max = s_max.max(pr.km);
                    d_min = d_min.min(pr.d);
                    d_max = d_max.max(pr.d);
                }
            }
            Corridor::Polygon {
                s_min,
                s_max,
                d_min,
                d_max,
            }
        };
        let radius = corridor.radius_m(km_c, d_c);
        for pi in grid.near(src.centroid, radius) {
            let poi = &pois[pi as usize];
            let Some(pr) = project(poi.pos) else {
                continue;
            };
            let in_polygon = src.rings.iter().any(|r| point_in_polygon(r, poi.pos));
            if !(in_polygon || corridor.contains(pr.km, pr.d)) {
                continue;
            }
            // Invariant, stage 1: filter (also for POIs inside the polygon).
            if pr.d <= 0.0 {
                result.excluded.push(ExcludedPoi {
                    src: si,
                    poi: pi as usize,
                    d: pr.d,
                    reason: "d <= 0 (not right of the strand)",
                });
                continue;
            }
            if let Some(near) = ctx.index.nearest(poi.pos) {
                if near.strand != chosen.strand
                    && is_opposite(strand, pr.segment, &strands[near.strand], near.proj.segment)
                {
                    result.excluded.push(ExcludedPoi {
                        src: si,
                        poi: pi as usize,
                        d: pr.d,
                        reason: "nearest strand is the opposite direction",
                    });
                    continue;
                }
            }
            claims.push(Claim {
                poi: pi as usize,
                fac: pl,
                score: (pr.d - d_c).abs(),
            });
        }
    }

    // A POI claimed by several facilities goes to the one with the smallest |d_poi - d_centroid|.
    claims.sort_by(|a, b| {
        a.poi
            .cmp(&b.poi)
            .then(a.score.total_cmp(&b.score))
            .then(a.fac.cmp(&b.fac))
    });
    let mut i = 0;
    while i < claims.len() {
        let mut j = i;
        while j < claims.len() && claims[j].poi == claims[i].poi {
            j += 1;
        }
        let winner = claims[i].fac;
        result.placed[winner].pois.push(claims[i].poi);
        if j - i > 1 {
            result.multi_claims.push(MultiClaim {
                poi: claims[i].poi,
                winner: result.placed[winner].src,
                others: claims[i + 1..j]
                    .iter()
                    .map(|c| result.placed[c.fac].src)
                    .collect(),
            });
        }
        i = j;
    }

    // Brands and amenities (§5, §6.4).
    let fast_idx = fast_brand_indices(ctx.brands);
    let mut unmatched: BTreeMap<String, (usize, BTreeSet<usize>)> = BTreeMap::new();
    for (pl, p) in result.placed.iter_mut().enumerate() {
        let src = &sources[p.src];
        // The services object itself: brand tag only.
        let m = ctx.matcher.match_object(src.tags.get("brand"), None, false);
        p.brands.extend(m.brands);
        for u in m.unmatched {
            let e = unmatched.entry(u).or_default();
            e.0 += 1;
            e.1.insert(pl);
        }
        p.shower |= amenities::has_shower_yes(&src.tags);
        p.open24h |= amenities::is_open_24_7(&src.tags);
        for &pi in &p.pois {
            let c = poi_contribution(ctx.matcher, &fast_idx, pois[pi].tags);
            p.fast_charger |= c.fast_charger;
            p.brands.extend(c.brands);
            for u in c.unmatched {
                let e = unmatched.entry(u).or_default();
                e.0 += 1;
                e.1.insert(pl);
            }
            p.shower |= c.shower;
            p.open24h |= c.gastro_open24h;
        }
    }
    result.unmatched_brands = unmatched;
    result
}

// ---------------------------------------------------------------------------------------------
// Brands and amenities of one POI (shared by Rastanlagen and Autohoefe)
// ---------------------------------------------------------------------------------------------

/// Indices of the brands that imply a fast charger (`FAST_CHARGER_BRANDS`).
pub fn fast_brand_indices(brands: &[Brand]) -> BTreeSet<usize> {
    brands
        .iter()
        .enumerate()
        .filter(|(_, b)| FAST_CHARGER_BRANDS.contains(&b.id.as_str()))
        .map(|(i, _)| i)
        .collect()
}

/// What one POI contributes to the facility it belongs to (heuristics §5, §6.4).
pub struct PoiContribution {
    pub brands: BTreeSet<usize>,
    pub unmatched: Vec<String>,
    pub shower: bool,
    pub fast_charger: bool,
    /// The POI is a gastronomy POI that is open 24/7.
    pub gastro_open24h: bool,
}

pub fn poi_contribution(
    matcher: &Matcher,
    fast_idx: &BTreeSet<usize>,
    tags: &Tags,
) -> PoiContribution {
    let is_charger = tags.get("amenity") == Some("charging_station");
    let mut m = matcher.match_object(tags.get("brand"), tags.get("name"), true);
    if is_charger && m.brands.is_empty() {
        if let Some(op) = tags.get("operator") {
            m.brands = matcher.match_operator(op);
        }
    }
    let fast_charger = is_charger
        && (amenities::is_fast_charger(tags) || m.brands.iter().any(|b| fast_idx.contains(b)));
    PoiContribution {
        brands: m.brands,
        unmatched: m.unmatched,
        shower: amenities::has_shower_yes(tags) || tags.get("amenity") == Some("shower"),
        fast_charger,
        gastro_open24h: amenities::is_gastronomy(tags) && amenities::is_open_24_7(tags),
    }
}

// ---------------------------------------------------------------------------------------------
// Dataset entries
// ---------------------------------------------------------------------------------------------

/// One facility entry of the dataset (`kind: rastanlage` or `autohof`).
#[derive(Clone, Debug)]
pub struct FacilityEntry {
    pub kind: &'static str,
    pub exit: Option<String>,
    pub detour_km: f64,
    pub id: String,
    pub name: Option<String>,
    pub strand_id: String,
    pub km: f64,
    pub lat: f64,
    pub lon: f64,
    pub brands: Vec<String>,
    pub shower: bool,
    pub fast_charger: bool,
    pub open24h: bool,
    /// Index into `Assignment::placed` (rastanlage) or `AutohofResult::placed` (autohof).
    pub placed: usize,
}

pub fn entries(
    a: &Assignment,
    sources: &[Source],
    strands: &[Strand],
    brands: &[Brand],
) -> Vec<FacilityEntry> {
    let mut out = Vec::new();
    for (pi, p) in a.placed.iter().enumerate() {
        let src = &sources[p.src];
        let name = src
            .tags
            .get("name")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let mut per_strand: Vec<(usize, f64)> = vec![(p.strand, p.proj.km)];
        per_strand.extend(p.extra.iter().copied());
        let multi = per_strand.len() > 1;
        for (sidx, km) in per_strand {
            let strand = &strands[sidx];
            let base = src.obj.base_id();
            out.push(FacilityEntry {
                kind: "rastanlage",
                exit: None,
                detour_km: 0.0,
                id: if multi {
                    format!("{base}@{}", strand.id)
                } else {
                    base
                },
                name: name.clone(),
                strand_id: strand.id.clone(),
                km: round_to(km, 3).min(strand.length_km),
                lat: round_to(src.centroid.lat, 5),
                lon: round_to(src.centroid.lon, 5),
                // Order as in brands.json = ascending index.
                brands: p.brands.iter().map(|&i| brands[i].id.clone()).collect(),
                shower: p.shower,
                fast_charger: p.fast_charger,
                open24h: p.open24h,
                placed: pi,
            });
        }
    }
    out.sort_by(|a, b| {
        a.strand_id
            .as_bytes()
            .cmp(b.strand_id.as_bytes())
            .then(a.km.total_cmp(&b.km))
            .then(a.id.as_bytes().cmp(b.id.as_bytes()))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brands;
    use crate::geo::local::cumulative_km;
    use crate::osm::model::{TaggedNode, TaggedWay};

    pub fn mk_strand(id: &str, ref_: &str, pts: Vec<LatLon>) -> Strand {
        let cum_km = cumulative_km(&pts);
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

    fn tags(kv: &[(&str, &str)]) -> Tags {
        Tags(
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    fn brand_list() -> Vec<Brand> {
        brands::parse(
            r#"[{"id":"aral","displayName":"Aral","category":"fuel","match":{"brand":["Aral"],"name":["Aral"]},"simpleIconsSlug":null},
                {"id":"burger-king","displayName":"BK","category":"food","match":{"brand":["Burger King"],"name":["Burger King"]},"simpleIconsSlug":null},
                {"id":"ionity","displayName":"Ionity","category":"charging","match":{"brand":["Ionity"],"name":["Ionity"]},"simpleIconsSlug":null},
                {"id":"tesla-supercharger","displayName":"Tesla Supercharger","category":"charging","match":{"brand":["Tesla Supercharger"],"name":["Tesla Supercharger"]},"simpleIconsSlug":null}]"#,
            "t",
        )
        .unwrap()
    }

    /// Model with nodes 1..: a polygon facility east of a northbound strand and POIs.
    fn add_poi(model: &mut Model, id: i64, lat: f64, lon: f64, kv: &[(&str, &str)]) {
        model.poi_nodes.insert(
            id,
            TaggedNode {
                pos: LatLon::new(lat, lon),
                tags: tags(kv),
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn add_square(
        model: &mut Model,
        id: i64,
        first_node: i64,
        lat0: f64,
        lon0: f64,
        lat1: f64,
        lon1: f64,
        kv: &[(&str, &str)],
    ) {
        let corners = [
            (lat0, lon0),
            (lat0, lon1),
            (lat1, lon1),
            (lat1, lon0),
            (lat0, lon0),
        ];
        let mut nodes = Vec::new();
        for (i, (la, lo)) in corners.iter().enumerate() {
            let nid = first_node + (i % 4) as i64;
            model.coords.insert(nid, LatLon::new(*la, *lo));
            let _ = i;
            nodes.push(nid);
        }
        model.service_ways.insert(
            id,
            TaggedWay {
                nodes,
                tags: tags(kv),
            },
        );
    }

    fn run_full(model: &Model, strands: &[Strand]) -> (Assignment, Vec<String>, Prepared) {
        let index = StrandIndex::new(strands);
        let b = brand_list();
        let matcher = Matcher::new(&b);
        let ctx = Context {
            strands,
            index: &index,
            matcher: &matcher,
            brands: &b,
        };
        let (all, _) = collect_sources(model);
        let ex: HashSet<ObjRef> = all.iter().map(|s| s.obj).collect();
        let pois = collect_pois(model, &ex);
        let prep = prepare(all, strands, &index);
        let a = assign(&prep.sources, &pois, &ctx);
        let ids = a
            .placed
            .iter()
            .flat_map(|p| p.pois.iter().map(|&i| pois[i].obj.base_id()))
            .collect();
        (a, ids, prep)
    }

    fn run(model: &Model, strands: &[Strand]) -> (Assignment, Vec<String>) {
        let (a, ids, _) = run_full(model, strands);
        (a, ids)
    }

    fn north_strand() -> Vec<Strand> {
        vec![mk_strand(
            "A1-N-0",
            "A 1",
            vec![LatLon::new(50.0, 10.0), LatLon::new(50.05, 10.0)],
        )]
    }

    fn service_node(m: &mut Model, id: i64, lat: f64, lon: f64, kv: &[(&str, &str)]) {
        let mut all = vec![("highway", "services")];
        all.extend_from_slice(kv);
        m.service_nodes.insert(
            id,
            TaggedNode {
                pos: LatLon::new(lat, lon),
                tags: tags(&all),
            },
        );
    }

    #[test]
    fn ring_assembly() {
        let r = assemble_rings(&[
            vec![1, 2, 3],
            vec![5, 4, 3],
            vec![5, 6, 1],
            vec![10, 11, 12, 10],
        ]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], vec![1, 2, 3, 4, 5, 6, 1]);
        assert_eq!(r[1], vec![10, 11, 12, 10]);
        // Unclosed leftovers are kept.
        assert_eq!(assemble_rings(&[vec![1, 2], vec![7, 8]]).len(), 2);
    }

    #[test]
    fn right_side_corridor_and_invariant() {
        // Northbound strand at lon 10.0, southbound at lon 9.9996 (west of it).
        let north = mk_strand(
            "A1-N-0",
            "A 1",
            vec![LatLon::new(50.0, 10.0), LatLon::new(50.05, 10.0)],
        );
        let south = mk_strand(
            "A1-S-0",
            "A 1",
            vec![LatLon::new(50.05, 9.9996), LatLon::new(50.0, 9.9996)],
        );
        let strands = vec![north, south];
        let mut m = Model::default();
        // Polygon east of both, 10.0010..10.0020 (71..142 m).
        add_square(
            &mut m,
            100,
            1,
            50.020,
            10.0010,
            50.021,
            10.0020,
            &[
                ("highway", "services"),
                ("name", "Test Rast"),
                ("brand", "Aral"),
            ],
        );
        // Inside the polygon, right of the northbound strand.
        add_poi(
            &mut m,
            1,
            50.0205,
            10.0015,
            &[
                ("amenity", "restaurant"),
                ("brand", "Burger King"),
                ("opening_hours", "24/7"),
            ],
        );
        // In the lateral buffer (<= 50 m behind the outer edge, along the polygon).
        add_poi(
            &mut m,
            2,
            50.0205,
            10.0023,
            &[("amenity", "charging_station"), ("socket:type2_combo", "2")],
        );
        // Too far outside (> 50 m).
        add_poi(&mut m, 3, 50.0205, 10.0035, &[("amenity", "shower")]);
        // Along the strand but beyond s_max.
        add_poi(&mut m, 4, 50.0230, 10.0015, &[("amenity", "shower")]);
        let (a, ids) = run(&m, &strands);
        assert_eq!(a.placed.len(), 1);
        let p = &a.placed[0];
        assert_eq!(strands[p.strand].id, "A1-N-0");
        assert!(p.extra.is_empty());
        assert_eq!(
            ids,
            vec!["osm-node-1".to_string(), "osm-node-2".to_string()]
        );
        assert!(p.fast_charger && p.open24h && !p.shower);
        assert_eq!(p.brands.iter().copied().collect::<Vec<_>>(), vec![0, 1]);
        assert!(
            a.excluded.is_empty(),
            "{:?}",
            a.excluded.iter().map(|e| e.reason).collect::<Vec<_>>()
        );
    }

    #[test]
    fn invariant_filter_excludes_left_and_opposite() {
        // Southbound strand EAST of the northbound one (left-hand arrangement), so that points
        // right of the northbound strand can be nearer to the opposite strand.
        let north = mk_strand(
            "A1-N-0",
            "A 1",
            vec![LatLon::new(50.0, 10.0), LatLon::new(50.05, 10.0)],
        );
        let south = mk_strand(
            "A1-S-0",
            "A 1",
            vec![LatLon::new(50.05, 10.0004), LatLon::new(50.0, 10.0004)],
        );
        let strands = vec![north, south];
        let mut m = Model::default();
        // Polygon overlapping both carriageways: lon 9.9990 .. 10.0020.
        add_square(
            &mut m,
            100,
            1,
            50.020,
            9.9990,
            50.021,
            10.0020,
            &[("highway", "services")],
        );
        // Centroid (lon 10.0005) is east of S (d<0 for S) and east of N (d>0): assigned to N.
        add_poi(&mut m, 1, 50.0205, 9.9995, &[("amenity", "cafe")]); // left of N -> d<0
        add_poi(&mut m, 2, 50.0205, 10.00035, &[("amenity", "cafe")]); // right of N, nearest S
        let (a, ids) = run(&m, &strands);
        assert!(ids.is_empty());
        let reasons: Vec<&str> = a.excluded.iter().map(|e| e.reason).collect();
        assert_eq!(reasons.len(), 2, "{reasons:?}");
        assert!(reasons.iter().any(|r| r.starts_with("d <= 0")));
        assert!(reasons.iter().any(|r| r.contains("opposite")));
    }

    #[test]
    fn unassigned_when_only_on_the_left() {
        let north = mk_strand(
            "A1-N-0",
            "A 1",
            vec![LatLon::new(50.0, 10.0), LatLon::new(50.05, 10.0)],
        );
        let strands = vec![north];
        let mut m = Model::default();
        m.service_nodes.insert(
            1,
            TaggedNode {
                pos: LatLon::new(50.02, 9.9990),
                tags: tags(&[("highway", "services")]),
            },
        );
        let (a, _) = run(&m, &strands);
        assert!(a.placed.is_empty());
        assert_eq!(a.unassigned.len(), 1);
        let (s, dist, d) = a.unassigned[0].nearest.unwrap();
        assert_eq!(s, 0);
        assert!(dist > 60.0 && dist < 80.0 && d < 0.0);
    }

    #[test]
    fn shared_section_emits_every_ref_with_suffix() {
        let a1 = mk_strand(
            "A1-N-0",
            "A 1",
            vec![LatLon::new(50.0, 10.0), LatLon::new(50.05, 10.0)],
        );
        let a3 = mk_strand(
            "A3-N-0",
            "A 3",
            vec![LatLon::new(50.0, 10.00002), LatLon::new(50.05, 10.00002)],
        );
        let a7 = mk_strand(
            "A7-N-0",
            "A 7",
            vec![LatLon::new(50.0, 9.999), LatLon::new(50.05, 9.999)],
        );
        let strands = vec![a1, a3, a7];
        let mut m = Model::default();
        m.service_nodes.insert(
            9,
            TaggedNode {
                pos: LatLon::new(50.02, 10.001),
                tags: tags(&[("highway", "services"), ("name", "Shared")]),
            },
        );
        let (a, _) = run(&m, &strands);
        assert_eq!(a.placed.len(), 1);
        // |d|: A3 70.1 m (smallest, chosen), A1 71.5 m (within 5 m), A7 143 m (outside).
        let p = &a.placed[0];
        assert_eq!(strands[p.strand].id, "A3-N-0");
        assert_eq!(p.extra.len(), 1);
        assert_eq!(strands[p.extra[0].0].id, "A1-N-0");
        let b = brand_list();
        let (sources, _) = collect_sources(&m);
        let e = entries(&a, &sources, &strands, &b);
        let ids: Vec<&str> = e.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["osm-node-9@A1-N-0", "osm-node-9@A3-N-0"]);
    }

    fn two_strands(south_lon: f64) -> Vec<Strand> {
        vec![
            mk_strand(
                "A1-N-0",
                "A 1",
                vec![LatLon::new(50.0, 10.0), LatLon::new(50.05, 10.0)],
            ),
            mk_strand(
                "A1-S-0",
                "A 1",
                vec![LatLon::new(50.05, south_lon), LatLon::new(50.0, south_lon)],
            ),
        ]
    }

    #[test]
    fn both_sides_node_nearer_to_opposite_strand() {
        // Left-hand arrangement: S east of N; node east of both: right of N, nearer to S.
        let strands = two_strands(10.0006);
        let mut m = Model::default();
        service_node(&mut m, 1, 50.02, 10.0010, &[]);
        let (a, _) = run(&m, &strands);
        assert_eq!(a.placed.len(), 1);
        assert_eq!(a.both_sides.len(), 1);
        assert!(a.both_sides[0].reason.contains("centroid"));
        assert_eq!(a.both_sides[0].strands.len(), 2);
    }

    #[test]
    fn both_sides_node_between_carriageways_is_not_both_sides() {
        // Node 36 m east of N, 72 m from S (right-hand arrangement): nearer to its own strand.
        let strands = two_strands(9.9995);
        let mut m = Model::default();
        service_node(&mut m, 1, 50.02, 10.0005, &[]);
        let (a, _) = run(&m, &strands);
        assert_eq!(a.placed.len(), 1);
        assert!(a.both_sides.is_empty());
    }

    #[test]
    fn both_sides_polygon_vertex_beyond_own_carriageway() {
        // Polygon reaches 28 m west of N (d < -15 m); S is 93 m from the centroid.
        let strands = two_strands(9.9990);
        let mut m = Model::default();
        add_square(
            &mut m,
            100,
            1,
            50.020,
            9.9996,
            50.021,
            10.0010,
            &[("highway", "services")],
        );
        let (a, _) = run(&m, &strands);
        assert_eq!(a.placed.len(), 1);
        assert_eq!(a.both_sides.len(), 1, "{:?}", a.unassigned.len());
        assert!(a.both_sides[0].reason.contains("vertex"));
    }

    #[test]
    fn both_sides_polygon_crosses_opposite_strand() {
        // Left-hand arrangement, polygon east of N spanning across S (lon 10.0004).
        let strands = two_strands(10.0004);
        let mut m = Model::default();
        add_square(
            &mut m,
            100,
            1,
            50.020,
            10.0002,
            50.021,
            10.0010,
            &[("highway", "services")],
        );
        let (a, _) = run(&m, &strands);
        assert_eq!(a.both_sides.len(), 1);
        assert!(a.both_sides[0].reason.contains("crosses"));
    }

    #[test]
    fn both_sides_negative_for_small_polygon_on_own_side() {
        // S only 14 m west of N, but the polygon stays east of N and nearer to N.
        let strands = two_strands(9.9998);
        let mut m = Model::default();
        add_square(
            &mut m,
            100,
            1,
            50.020,
            10.0003,
            50.021,
            10.0006,
            &[("highway", "services")],
        );
        let (a, _) = run(&m, &strands);
        assert_eq!(a.placed.len(), 1);
        assert!(a.both_sides.is_empty());
    }

    #[test]
    fn autohof_like_objects_are_deferred() {
        let strands = north_strand();
        let mut m = Model::default();
        service_node(&mut m, 1, 50.02, 10.0010, &[("name", "MAXI Autohof Nord")]);
        service_node(&mut m, 2, 50.03, 10.0010, &[("operator", "Truckstop GmbH")]);
        service_node(&mut m, 3, 50.04, 10.0010, &[("name", "Rasthof Ost")]);
        let (a, _, prep) = run_full(&m, &strands);
        assert_eq!(prep.deferred.len(), 2);
        assert_eq!(prep.deferred[0].obj.id, 1);
        assert!(prep.deferred[0].reason.contains("autohof"));
        assert!(prep.deferred[1].reason.contains("operator"));
        assert_eq!(prep.sources.len(), 1);
        assert_eq!(a.placed.len(), 1);
        assert!(a.unassigned.is_empty());
    }

    #[test]
    fn merge_happens_before_the_autohof_like_exclusion() {
        let strands = north_strand();
        let mut m = Model::default();
        // A plain polygon (would be a Rastanlage) with an "Autohof" node inside: after merging,
        // the whole site is Autohof-like and deferred, not a Rastanlage plus an Autohof.
        add_square(
            &mut m,
            100,
            1,
            50.020,
            10.0010,
            50.021,
            10.0020,
            &[("highway", "services")],
        );
        service_node(&mut m, 1, 50.0205, 10.0015, &[("name", "Autohof Mitte")]);
        // Control: a plain polygon without Autohof stays a Rastanlage source.
        add_square(
            &mut m,
            200,
            11,
            50.030,
            10.0010,
            50.031,
            10.0020,
            &[("highway", "services")],
        );
        let (a, _, prep) = run_full(&m, &strands);
        assert_eq!(prep.merged.len(), 1);
        assert_eq!(prep.deferred.len(), 1);
        assert_eq!(prep.deferred[0].obj.id, 100);
        assert_eq!(
            prep.deferred[0].source.tags.get("name"),
            Some("Autohof Mitte")
        );
        assert_eq!(prep.sources.len(), 1);
        assert_eq!(prep.sources[0].obj.id, 200);
        assert_eq!(a.placed.len(), 1);
    }

    #[test]
    fn far_objects_are_deferred_near_ones_kept() {
        let strands = north_strand();
        let mut m = Model::default();
        // 10.0010 = 71 m, 10.0022 = 157 m, 10.0020 = 143 m east of the strand.
        service_node(&mut m, 1, 50.01, 10.0010, &[]);
        service_node(&mut m, 2, 50.02, 10.0022, &[]);
        service_node(&mut m, 3, 50.03, 10.0020, &[]);
        let (a, _, prep) = run_full(&m, &strands);
        assert_eq!(prep.deferred.len(), 1);
        assert_eq!(prep.deferred[0].obj.id, 2);
        assert!(prep.deferred[0].reason.starts_with("|d|"));
        assert_eq!(a.placed.len(), 2);
    }

    #[test]
    fn duplicates_are_merged_polygon_kept_and_tags_filled() {
        let strands = north_strand();
        let mut m = Model::default();
        add_square(
            &mut m,
            100,
            1,
            50.020,
            10.0010,
            50.021,
            10.0020,
            &[("highway", "services")],
        );
        // Nodes inside the polygon: one with name/brand/shower, one with opening_hours.
        service_node(
            &mut m,
            1,
            50.0205,
            10.0015,
            &[("name", "Inner"), ("brand", "Aral"), ("shower", "yes")],
        );
        service_node(
            &mut m,
            2,
            50.0203,
            10.0012,
            &[("name", "Other"), ("opening_hours", "24/7")],
        );
        // Node next to the polygon but outside: stays separate.
        service_node(&mut m, 3, 50.0212, 10.0020, &[("name", "Outside")]);
        let (a, _, prep) = run_full(&m, &strands);
        assert_eq!(prep.merged.len(), 1);
        let g = &prep.merged[0];
        assert_eq!((g.kept.typ, g.kept.id), (ObjType::Way, 100));
        assert_eq!(g.dropped.len(), 2);
        assert_eq!(prep.sources.len(), 2);
        assert_eq!(a.placed.len(), 2);
        let kept = prep.sources.iter().find(|s| s.obj.id == 100).unwrap();
        assert_eq!(kept.tags.get("name"), Some("Inner")); // first dropped object wins
        assert_eq!(kept.tags.get("brand"), Some("Aral"));
        assert_eq!(kept.tags.get("shower"), Some("yes"));
        assert_eq!(kept.tags.get("opening_hours"), Some("24/7"));
        let p = a
            .placed
            .iter()
            .find(|p| prep.sources[p.src].obj.id == 100)
            .unwrap();
        assert!(p.shower && p.open24h && p.brands.contains(&0));
    }

    #[test]
    fn merge_keeps_larger_polygon_and_does_not_overwrite_tags() {
        let strands = north_strand();
        let mut m = Model::default();
        add_square(
            &mut m,
            100,
            1,
            50.020,
            10.0010,
            50.021,
            10.0020,
            &[("highway", "services"), ("name", "Small")],
        );
        // Larger polygon containing the small one's centroid.
        add_square(
            &mut m,
            200,
            11,
            50.0195,
            10.0008,
            50.0215,
            10.0022,
            &[("highway", "services"), ("name", "Large")],
        );
        let (_, _, prep) = run_full(&m, &strands);
        assert_eq!(prep.merged.len(), 1);
        assert_eq!(prep.merged[0].kept.id, 200);
        assert_eq!(prep.sources[0].tags.get("name"), Some("Large"));
    }

    #[test]
    fn operator_fallback_only_for_chargers() {
        let strands = north_strand();
        let mut m = Model::default();
        service_node(&mut m, 1, 50.02, 10.0010, &[]);
        // Charger without brand: operator resolves the brand and implies fastCharger.
        add_poi(
            &mut m,
            101,
            50.0201,
            10.0011,
            &[("amenity", "charging_station"), ("operator", "Ionity")],
        );
        // Restaurant with operator "Aral": operator is ignored.
        add_poi(
            &mut m,
            102,
            50.0202,
            10.0011,
            &[("amenity", "restaurant"), ("operator", "Aral")],
        );
        let (a, _) = run(&m, &strands);
        let p = &a.placed[0];
        assert_eq!(p.brands.iter().copied().collect::<Vec<_>>(), vec![2]);
        assert!(p.fast_charger);
    }

    #[test]
    fn generic_tesla_brand_is_not_a_supercharger() {
        let strands = north_strand();
        let mut m = Model::default();
        service_node(&mut m, 1, 50.02, 10.0010, &[]);
        add_poi(
            &mut m,
            101,
            50.0201,
            10.0011,
            &[("amenity", "charging_station"), ("brand", "Tesla")],
        );
        let (a, _) = run(&m, &strands);
        let p = &a.placed[0];
        assert!(p.brands.is_empty());
        assert!(!p.fast_charger);
        assert_eq!(a.unmatched_brands.get("Tesla").map(|e| e.0), Some(1));
        // The actual brand value does imply a fast charger.
        let mut m = Model::default();
        service_node(&mut m, 1, 50.02, 10.0010, &[]);
        add_poi(
            &mut m,
            101,
            50.0201,
            10.0011,
            &[
                ("amenity", "charging_station"),
                ("brand", "Tesla Supercharger"),
            ],
        );
        let (a, _) = run(&m, &strands);
        assert!(a.placed[0].fast_charger);
        assert_eq!(
            a.placed[0].brands.iter().copied().collect::<Vec<_>>(),
            vec![3]
        );
    }

    #[test]
    fn open24h_counts_gastronomy_spellings_but_not_fuel() {
        let strands = north_strand();
        let mut m = Model::default();
        service_node(&mut m, 1, 50.02, 10.0010, &[]);
        add_poi(
            &mut m,
            101,
            50.0201,
            10.0011,
            &[("amenity", "fuel"), ("opening_hours", "24/7")],
        );
        let (a, _) = run(&m, &strands);
        assert!(!a.placed[0].open24h);
        add_poi(
            &mut m,
            102,
            50.0202,
            10.0011,
            &[("amenity", "cafe"), ("opening_hours", "Mo-Su 00:00-24:00")],
        );
        let (a, _) = run(&m, &strands);
        assert!(a.placed[0].open24h);
    }

    #[test]
    fn poi_claimed_twice_goes_to_the_better_facility() {
        let north = mk_strand(
            "A1-N-0",
            "A 1",
            vec![LatLon::new(50.0, 10.0), LatLon::new(50.1, 10.0)],
        );
        let strands = vec![north];
        let mut m = Model::default();
        for (id, lat) in [(1, 50.0200), (2, 50.0204)] {
            m.service_nodes.insert(
                id,
                TaggedNode {
                    pos: LatLon::new(lat, 10.0010),
                    tags: tags(&[("highway", "services")]),
                },
            );
        }
        // Within 150 m of both node facilities, closer in |d| terms to facility 2 (d 80 m vs 71).
        add_poi(&mut m, 200, 50.0203, 10.0011, &[("amenity", "cafe")]);
        let (a, _) = run(&m, &strands);
        assert_eq!(a.multi_claims.len(), 1);
        let owner: Vec<usize> = a.placed.iter().map(|p| p.pois.len()).collect();
        assert_eq!(owner.iter().sum::<usize>(), 1);
    }
}
