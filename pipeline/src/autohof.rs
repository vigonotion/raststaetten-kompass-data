//! Autohoefe (heuristics §7): junction clusters, candidates, qualification, one Autohof per
//! cluster, one dataset entry per strand of the cluster.

use crate::amenities;
use crate::brands::{Brand, Matcher};
use crate::facilities::{
    fast_brand_indices, poi_contribution, Deferred, FacilityEntry, ObjRef, Poi, PoiGrid, Source,
};
use crate::geo::distance::haversine_m;
use crate::geo::polygon::point_in_polygon;
use crate::geo::{round_to, LatLon};
use crate::osm::model::Tags;
use crate::params::*;
use crate::strands::{display_exit, normalize_list, Junction, Strand};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// One `motorway_junction` node on one strand.
#[derive(Clone, Debug)]
pub struct JNode {
    pub strand: usize,
    pub km: f64,
    pub pos: LatLon,
}

/// An "Anschlussstelle": junction nodes of one motorway with the same normalized (`ref`, `name`)
/// (D-33: `;` lists split, trimmed, sorted) that lie at most 1.5 km apart (transitively).
#[derive(Clone, Debug)]
pub struct Cluster {
    pub motorway: String,
    /// Normalized key part of the name.
    pub jname: Option<String>,
    /// Display text (schema §2.3) from the first node's original `ref`/`name`.
    pub exit: String,
    pub nodes: Vec<JNode>,
}

impl Cluster {
    /// Rest-area exit (`Rastplatz`, `Raststätte`, ...): not an Anschlussstelle (D-32).
    pub fn is_rest_area(&self) -> bool {
        self.jname.as_deref().is_some_and(|names| {
            names.split(';').any(|n| {
                let n = n.trim().to_lowercase();
                REST_AREA_JUNCTION_PREFIXES.iter().any(|p| n.starts_with(p))
            })
        })
    }

    /// Display text of the junction (schema §2.3): `"<ref> <name>"`, or whichever exists.
    pub fn exit_text(&self) -> String {
        self.exit.clone()
    }
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent[ra.max(rb)] = ra.min(rb);
    }
}

/// Builds the junction clusters. Deterministic: ordered by (motorway, ref, name, first node).
pub fn build_clusters(strands: &[Strand], junctions: &[Junction]) -> Vec<Cluster> {
    let idx: HashMap<&str, usize> = strands
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();
    type Key = (String, Option<String>, Option<String>);
    let mut groups: BTreeMap<Key, Vec<(JNode, &Junction)>> = BTreeMap::new();
    for j in junctions {
        let Some(&si) = idx.get(j.strand_id.as_str()) else {
            continue;
        };
        groups
            .entry((
                strands[si].ref_.clone(),
                j.ref_.as_deref().map(normalize_list),
                j.name.as_deref().map(normalize_list),
            ))
            .or_default()
            .push((
                JNode {
                    strand: si,
                    km: j.km,
                    pos: j.pos,
                },
                j,
            ));
    }
    let mut out = Vec::new();
    for ((motorway, _jref, jname), nodes) in groups {
        let mut parent: Vec<usize> = (0..nodes.len()).collect();
        for a in 0..nodes.len() {
            for b in a + 1..nodes.len() {
                if haversine_m(nodes[a].0.pos, nodes[b].0.pos)
                    <= JUNCTION_CLUSTER_DISTANCE_KM * 1000.0
                {
                    union(&mut parent, a, b);
                }
            }
        }
        let mut comps: BTreeMap<usize, Vec<(JNode, &Junction)>> = BTreeMap::new();
        for (i, n) in nodes.iter().enumerate() {
            let r = find(&mut parent, i);
            comps.entry(r).or_default().push(n.clone());
        }
        for (_, nodes) in comps {
            let exit = display_exit(nodes[0].1.ref_.as_deref(), nodes[0].1.name.as_deref());
            out.push(Cluster {
                motorway: motorway.clone(),
                jname: jname.clone(),
                exit,
                nodes: nodes.into_iter().map(|(n, _)| n).collect(),
            });
        }
    }
    out
}

/// A candidate: a fuel station, or a deferred services object (with a merged-in fuel station).
#[derive(Clone, Debug)]
pub struct Cand {
    /// Object that gives ID and (primarily) name: the services object if there is one (D-31).
    pub obj: ObjRef,
    pub name: Option<String>,
    /// Position: the fuel station if present, else the centroid of the services object.
    pub pos: LatLon,
    /// Tags of the services object, if the candidate is a deferred services object.
    pub services_tags: Option<Tags>,
    /// The fuel station (merged into a services object or standalone).
    pub fuel: Option<(ObjRef, Tags)>,
}

impl Cand {
    pub fn is_deferred(&self) -> bool {
        self.services_tags.is_some()
    }
}

/// Why a candidate counts as Autohof.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reasons {
    /// Rule 1: deferred services object.
    pub deferred: bool,
    /// Rule 2: `Autohof` keyword in name/operator/brand.
    pub keyword: bool,
    /// Rule 3 (candidate part): gastronomy POI <= 150 m and truck parking <= 300 m. The junction
    /// distance (<= 1 km) is checked per cluster.
    pub gastro: bool,
}

impl Reasons {
    /// Rank within a cluster: rule 1 (0) > rule 2 (1) > rule 3 (2).
    pub fn strength(&self) -> u8 {
        if self.deferred {
            0
        } else if self.keyword {
            1
        } else {
            2
        }
    }

    /// Whether the candidate may serve a cluster at `dist_m` (nearest node) per D-32: `Rast*`
    /// clusters only for deferred services objects; rule 3 only up to 1 km.
    pub fn allowed_at(&self, rest_area_cluster: bool, dist_m: f64) -> bool {
        if self.deferred {
            return true;
        }
        if rest_area_cluster {
            return false;
        }
        self.keyword || dist_m <= AUTOHOF_RULE3_MAX_JUNCTION_KM * 1000.0
    }
}

impl Reasons {
    pub fn any(&self) -> bool {
        self.deferred || self.keyword || self.gastro
    }
    pub fn label(&self) -> String {
        let mut v = Vec::new();
        if self.deferred {
            v.push("deferred services object");
        }
        if self.keyword {
            v.push("Autohof keyword");
        }
        if self.gastro {
            v.push("gastronomy POI <= 150 m + truck parking <= 300 m");
        }
        v.join(" + ")
    }
}

/// `Autohof`, `Truckstop` or `Truck Stop` (case-insensitive, also inside words) in `name`,
/// `operator` or `brand` (rule 2).
pub fn has_autohof_keyword(tags: &Tags) -> bool {
    ["name", "operator", "brand"].iter().any(|k| {
        tags.get(k).is_some_and(|v| {
            let v = v.to_lowercase();
            AUTOHOF_RULE2_KEYWORDS.iter().any(|kw| v.contains(kw))
        })
    })
}

/// A chosen Autohof for one cluster.
pub struct AutohofPlaced {
    pub cand: usize,
    pub cluster: usize,
    /// Distance to the nearest node of the cluster in metres.
    pub dist_m: f64,
    pub reasons: Reasons,
    pub brands: BTreeSet<usize>,
    pub shower: bool,
    pub fast_charger: bool,
    pub open24h: bool,
    /// POIs within 150 m (indices into the POI list).
    pub pois: Vec<usize>,
}

/// A qualified candidate that did not become an Autohof of the cluster.
pub struct Additional {
    pub cand: usize,
    pub cluster: usize,
    pub dist_m: f64,
    pub why: String,
}

/// An Autohof that was removed after picking (D-33).
pub struct Dropped {
    pub cand: usize,
    pub cluster: usize,
    pub why: String,
}

#[derive(Default, Debug)]
pub struct Counts {
    /// Autohöfe removed as duplicate sites (D-33).
    pub site_duplicates: usize,
    /// Autohöfe removed because they lie within 150 m of a Rastanlage centroid (D-33).
    pub at_rastanlage: usize,
    pub clusters: usize,
    pub fuel_total: usize,
    pub fuel_assigned: usize,
    pub fuel_invariant_excluded: usize,
    pub fuel_in_rastanlage_polygon: usize,
    pub fuel_merged_into_services: usize,
    /// Candidates (fuel stations and services objects) within 2 km of a junction node.
    pub candidates: usize,
    pub deferred_candidates: usize,
    pub q_deferred: usize,
    pub q_keyword: usize,
    pub q_gastro: usize,
    pub qualified: usize,
    pub entries: usize,
    pub distinct: usize,
}

pub struct AutohofResult {
    pub clusters: Vec<Cluster>,
    pub cands: Vec<Cand>,
    pub placed: Vec<AutohofPlaced>,
    pub entries: Vec<FacilityEntry>,
    pub additional: Vec<Additional>,
    /// Deferred services objects farther than 2 km from every junction node.
    pub unplaced: Vec<usize>,
    /// Autohöfe removed by the site dedupe or because they lie at a Rastanlage (D-33).
    pub dropped: Vec<Dropped>,
    pub counts: Counts,
}

pub struct AutohofInput<'a> {
    pub strands: &'a [Strand],
    pub junctions: &'a [Junction],
    pub pois: &'a [Poi<'a>],
    /// POIs assigned to a Rastanlage.
    pub assigned: &'a HashSet<usize>,
    /// POIs that lie in the corridor of a Rastanlage but were excluded by the invariant filter.
    pub invariant_excluded: &'a HashSet<usize>,
    /// Kept Rastanlage sources (a fuel station inside their polygon is not a candidate).
    pub rast_sources: &'a [Source],
    /// Centroids of the Rastanlagen that are actually in the dataset (assigned to a strand).
    pub rastanlagen: &'a [(ObjRef, LatLon)],
    pub deferred: &'a [Deferred],
    pub matcher: &'a Matcher,
    pub brands: &'a [Brand],
    /// Truck parkings (`collect_truck_parkings`).
    pub truck_parkings: &'a [LatLon],
}

fn tags_name(tags: &Tags) -> Option<String> {
    tags.get("name")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Detour in km: straight line times the factor, 1 decimal, never below 0.1 (schema: > 0).
pub fn detour_km(dist_m: f64) -> f64 {
    round_to(dist_m / 1000.0 * AUTOHOF_DETOUR_FACTOR, 1).max(0.1)
}

/// Distances (m) from `pos` to every cluster within `AUTOHOF_MAX_DISTANCE_KM`, using a latitude
/// sorted list of all cluster nodes.
struct NodeIndex {
    /// (lat, cluster, position), sorted by latitude.
    nodes: Vec<(f64, usize, LatLon)>,
}

impl NodeIndex {
    fn new(clusters: &[Cluster]) -> Self {
        let mut nodes: Vec<(f64, usize, LatLon)> = clusters
            .iter()
            .enumerate()
            .flat_map(|(ci, c)| c.nodes.iter().map(move |n| (n.pos.lat, ci, n.pos)))
            .collect();
        nodes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        NodeIndex { nodes }
    }

    fn near(&self, pos: LatLon) -> Vec<(usize, f64)> {
        let dlat = AUTOHOF_MAX_DISTANCE_KM * 1000.0 / 110_000.0 * 1.05;
        let lo = self.nodes.partition_point(|n| n.0 < pos.lat - dlat);
        let mut best: BTreeMap<usize, f64> = BTreeMap::new();
        for n in &self.nodes[lo..] {
            if n.0 > pos.lat + dlat {
                break;
            }
            let d = haversine_m(n.2, pos);
            if d <= AUTOHOF_MAX_DISTANCE_KM * 1000.0 {
                let e = best.entry(n.1).or_insert(f64::INFINITY);
                *e = e.min(d);
            }
        }
        best.into_iter().collect()
    }
}

pub fn build(inp: &AutohofInput<'_>) -> AutohofResult {
    let clusters = build_clusters(inp.strands, inp.junctions);
    let nidx = NodeIndex::new(&clusters);
    let mut counts = Counts {
        clusters: clusters.len(),
        ..Counts::default()
    };

    // ---- Candidates ----
    // Fuel stations that are not part of a Rastanlage.
    let mut free_fuel: Vec<usize> = Vec::new();
    for (i, p) in inp.pois.iter().enumerate() {
        if p.tags.get("amenity") != Some("fuel") {
            continue;
        }
        counts.fuel_total += 1;
        if inp.assigned.contains(&i) {
            counts.fuel_assigned += 1;
        } else if inp.invariant_excluded.contains(&i) {
            counts.fuel_invariant_excluded += 1;
        } else if inp
            .rast_sources
            .iter()
            .any(|s| s.rings.iter().any(|r| point_in_polygon(r, p.pos)))
        {
            counts.fuel_in_rastanlage_polygon += 1;
        } else {
            free_fuel.push(i);
        }
    }
    // Merge fuel stations into deferred services objects (D-31): inside the polygon, or at most
    // 150 m from a services node. The nearest services object (to its centroid) wins.
    let mut merged_fuel: Vec<Vec<usize>> = vec![Vec::new(); inp.deferred.len()];
    let mut standalone: Vec<usize> = Vec::new();
    for &fi in &free_fuel {
        let pos = inp.pois[fi].pos;
        let mut best: Option<(f64, usize)> = None;
        for (di, d) in inp.deferred.iter().enumerate() {
            let src = &d.source;
            let hit = if src.rings.is_empty() {
                haversine_m(src.centroid, pos) <= AUTOHOF_POI_RADIUS_M
            } else {
                src.rings.iter().any(|r| point_in_polygon(r, pos))
            };
            if hit {
                let dist = haversine_m(src.centroid, pos);
                if best.is_none_or(|(bd, _)| dist < bd) {
                    best = Some((dist, di));
                }
            }
        }
        match best {
            Some((_, di)) => {
                merged_fuel[di].push(fi);
                counts.fuel_merged_into_services += 1;
            }
            None => standalone.push(fi),
        }
    }

    let mut cands: Vec<Cand> = Vec::new();
    let mut unplaced: Vec<usize> = Vec::new();
    let mut in_range: Vec<Vec<(usize, f64)>> = Vec::new();
    for (di, d) in inp.deferred.iter().enumerate() {
        let src = &d.source;
        // Nearest merged fuel station to the centroid is the position.
        let fuel = merged_fuel[di].iter().copied().min_by(|&a, &b| {
            haversine_m(src.centroid, inp.pois[a].pos)
                .total_cmp(&haversine_m(src.centroid, inp.pois[b].pos))
                .then(a.cmp(&b))
        });
        let (pos, fuel_pair) = match fuel {
            Some(fi) => (
                inp.pois[fi].pos,
                Some((inp.pois[fi].obj, inp.pois[fi].tags.clone())),
            ),
            None => (src.centroid, None),
        };
        let name = tags_name(&src.tags).or_else(|| {
            fuel_pair
                .as_ref()
                .and_then(|(_, t): &(ObjRef, Tags)| tags_name(t))
        });
        let near = nidx.near(pos);
        if near.is_empty() {
            unplaced.push(cands.len());
        }
        cands.push(Cand {
            obj: src.obj,
            name,
            pos,
            services_tags: Some(src.tags.clone()),
            fuel: fuel_pair,
        });
        in_range.push(near);
    }
    let first_fuel_cand = cands.len();
    for &fi in &standalone {
        let p = &inp.pois[fi];
        let near = nidx.near(p.pos);
        if near.is_empty() {
            continue;
        }
        cands.push(Cand {
            obj: p.obj,
            name: tags_name(p.tags),
            pos: p.pos,
            services_tags: None,
            fuel: Some((p.obj, p.tags.clone())),
        });
        in_range.push(near);
    }
    // Deferred candidates without any junction in range are "unplaced", not candidates.
    let unplaced_set: HashSet<usize> = unplaced.iter().copied().collect();
    counts.candidates = cands.len() - unplaced_set.len();
    counts.deferred_candidates = first_fuel_cand - unplaced_set.len();

    // ---- Qualification ----
    let grid = PoiGrid::new(inp.pois);
    let near_pois = |pos: LatLon| -> Vec<usize> {
        grid.near(pos, AUTOHOF_POI_RADIUS_M)
            .into_iter()
            .map(|i| i as usize)
            .filter(|&i| haversine_m(inp.pois[i].pos, pos) <= AUTOHOF_POI_RADIUS_M)
            .collect()
    };
    let parking_grid = {
        let mut cells: HashMap<crate::index::Cell, Vec<u32>> = HashMap::new();
        for (i, p) in inp.truck_parkings.iter().enumerate() {
            cells
                .entry(crate::index::cell_of(*p))
                .or_default()
                .push(i as u32);
        }
        cells
    };
    let truck_parking_near = |pos: LatLon| -> bool {
        let r = AUTOHOF_TRUCK_PARKING_RADIUS_M;
        let dlat = r / 110_000.0;
        let dlon = r / (110_000.0 * pos.lat.to_radians().cos().max(0.1));
        let (y0, x0) = crate::index::cell_of(LatLon::new(pos.lat - dlat, pos.lon - dlon));
        let (y1, x1) = crate::index::cell_of(LatLon::new(pos.lat + dlat, pos.lon + dlon));
        (y0..=y1).any(|y| {
            (x0..=x1).any(|x| {
                parking_grid.get(&(y, x)).is_some_and(|v| {
                    v.iter()
                        .any(|&i| haversine_m(inp.truck_parkings[i as usize], pos) <= r)
                })
            })
        })
    };
    let mut reasons: Vec<Reasons> = vec![Reasons::default(); cands.len()];
    let mut pois_of: Vec<Vec<usize>> = vec![Vec::new(); cands.len()];
    for (ci, c) in cands.iter().enumerate() {
        if in_range[ci].is_empty() {
            continue;
        }
        let np = near_pois(c.pos);
        let r = Reasons {
            deferred: c.is_deferred(),
            keyword: c.fuel.as_ref().is_some_and(|(_, t)| has_autohof_keyword(t)),
            gastro: np
                .iter()
                .any(|&i| amenities::is_gastronomy(inp.pois[i].tags))
                && truck_parking_near(c.pos),
        };
        counts.q_deferred += usize::from(r.deferred);
        counts.q_keyword += usize::from(r.keyword);
        counts.q_gastro += usize::from(r.gastro);
        counts.qualified += usize::from(r.any());
        reasons[ci] = r;
        pois_of[ci] = np;
    }

    // ---- One Autohof per cluster ----
    // Eligible candidates of each cluster: strongest rule first, then nearest (ties: candidate order).
    let mut per_cluster: Vec<Vec<(f64, usize)>> = vec![Vec::new(); clusters.len()];
    for (ci, near) in in_range.iter().enumerate() {
        if !reasons[ci].any() {
            continue;
        }
        for &(cl, d) in near {
            if reasons[ci].allowed_at(clusters[cl].is_rest_area(), d) {
                per_cluster[cl].push((d, ci));
            }
        }
    }
    let mut additional: Vec<Additional> = Vec::new();
    // candidate -> clusters that chose it as their nearest Autohof
    let mut picks: BTreeMap<usize, Vec<(usize, f64)>> = BTreeMap::new();
    for (cl, list) in per_cluster.iter_mut().enumerate() {
        list.sort_by(|a, b| {
            reasons[a.1]
                .strength()
                .cmp(&reasons[b.1].strength())
                .then(a.0.total_cmp(&b.0))
                .then(a.1.cmp(&b.1))
        });
        for (rank, &(d, ci)) in list.iter().enumerate() {
            if rank == 0 {
                picks.entry(ci).or_default().push((cl, d));
            } else {
                additional.push(Additional {
                    cand: ci,
                    cluster: cl,
                    dist_m: d,
                    why: format!(
                        "weaker or farther than the chosen Autohof of this cluster ({:.0} m)",
                        list[0].0
                    ),
                });
            }
        }
    }
    // Nearest site per strand (D-33): of the clusters that picked a candidate, each strand
    // keeps the cluster whose node on that strand is nearest to the candidate.
    let mut kept: Vec<(usize, usize, f64, Vec<usize>)> = Vec::new(); // (cand, cluster, dist, strands)
    for (ci, list) in &picks {
        let mut best: BTreeMap<usize, (f64, usize)> = BTreeMap::new(); // strand -> (dist, cluster)
        for &(cl, _) in list {
            for n in &clusters[cl].nodes {
                let d = haversine_m(n.pos, cands[*ci].pos);
                let e = best.entry(n.strand).or_insert((f64::INFINITY, cl));
                if d < e.0 || (d == e.0 && cl < e.1) {
                    *e = (d, cl);
                }
            }
        }
        for &(cl, d) in list {
            let strands: Vec<usize> = best
                .iter()
                .filter(|(_, v)| v.1 == cl)
                .map(|(&st, _)| st)
                .collect();
            if !strands.is_empty() {
                kept.push((*ci, cl, d, strands));
            } else {
                additional.push(Additional {
                    cand: *ci,
                    cluster: cl,
                    dist_m: d,
                    why: "also the pick of this cluster, but every strand has a nearer exit for it"
                        .to_string(),
                });
            }
        }
    }
    kept.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut dropped: Vec<Dropped> = Vec::new();
    // An Autohof at a Rastanlage is the Rastanlage (D-33).
    kept.retain(|(ci, cl, _, _)| {
        let near = inp
            .rastanlagen
            .iter()
            .map(|(obj, c)| (haversine_m(*c, cands[*ci].pos), *obj))
            .filter(|(d, _)| *d <= AUTOHOF_SITE_DEDUPE_M)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((d, obj)) = near {
            counts.at_rastanlage += 1;
            dropped.push(Dropped {
                cand: *ci,
                cluster: *cl,
                why: format!(
                    "{d:.0} m from the centroid of Rastanlage {}/{}",
                    obj.typ.as_str(),
                    obj.id
                ),
            });
        }
        near.is_none()
    });
    additional.sort_by(|a, b| a.cluster.cmp(&b.cluster).then(a.cand.cmp(&b.cand)));

    // ---- Brands, amenities, entries ----
    let fast_idx = fast_brand_indices(inp.brands);
    let mut placed: Vec<AutohofPlaced> = Vec::new();
    let mut entries: Vec<FacilityEntry> = Vec::new();
    for (ci, cl, dist, strand_set) in &kept {
        let (ci, cl, dist) = (*ci, *cl, *dist);
        let c = &cands[ci];
        let mut brands: BTreeSet<usize> = BTreeSet::new();
        let (mut shower, mut fast, mut open24h) = (false, false, false);
        if let Some(t) = &c.services_tags {
            brands.extend(inp.matcher.match_object(t.get("brand"), None, false).brands);
            shower |= amenities::has_shower_yes(t);
            open24h |= amenities::is_open_24_7(t);
        }
        for &pi in &pois_of[ci] {
            let pc = poi_contribution(inp.matcher, &fast_idx, inp.pois[pi].tags);
            brands.extend(pc.brands);
            shower |= pc.shower;
            fast |= pc.fast_charger;
            open24h |= pc.gastro_open24h;
        }
        let pl = placed.len();
        placed.push(AutohofPlaced {
            cand: ci,
            cluster: cl,
            dist_m: dist,
            reasons: reasons[ci],
            brands: brands.clone(),
            shower,
            fast_charger: fast,
            open24h,
            pois: pois_of[ci].clone(),
        });
        // One entry per strand of the cluster (junction node nearest to the position).
        let mut per_strand: BTreeMap<usize, (f64, &JNode)> = BTreeMap::new();
        for n in clusters[cl]
            .nodes
            .iter()
            .filter(|n| strand_set.contains(&n.strand))
        {
            let d = haversine_m(n.pos, c.pos);
            let better = per_strand
                .get(&n.strand)
                .is_none_or(|(bd, bn)| d < *bd || (d == *bd && n.km < bn.km));
            if better {
                per_strand.insert(n.strand, (d, n));
            }
        }
        let exit = clusters[cl].exit_text();
        let base = c.obj.base_id();
        for (si, (d, n)) in per_strand {
            let strand = &inp.strands[si];
            entries.push(FacilityEntry {
                kind: "autohof",
                exit: Some(exit.clone()),
                detour_km: detour_km(d),
                id: format!("{base}@{}", strand.id),
                name: c.name.clone(),
                strand_id: strand.id.clone(),
                km: round_to(n.km, 3).min(strand.length_km),
                lat: round_to(c.pos.lat, 5),
                lon: round_to(c.pos.lon, 5),
                brands: brands.iter().map(|&i| inp.brands[i].id.clone()).collect(),
                shower,
                fast_charger: fast,
                open24h,
                placed: pl,
            });
        }
    }
    // Site dedupe (D-33): candidates with entries on the same strand and positions at most 150 m
    // apart are one site; the one with the smallest detourKm stays (ties: smaller type/id).
    {
        let strand_ids: Vec<&str> = inp.strands.iter().map(|s| s.id.as_str()).collect();
        let strand_of = |e: &FacilityEntry| strand_ids.iter().position(|s| *s == e.strand_id);
        let mut by_cand: BTreeMap<usize, Vec<usize>> = BTreeMap::new(); // cand -> placed
        for (pl, p) in placed.iter().enumerate() {
            by_cand.entry(p.cand).or_default().push(pl);
        }
        let key = |ci: usize| -> f64 {
            entries
                .iter()
                .filter(|e| by_cand[&ci].contains(&e.placed))
                .map(|e| e.detour_km)
                .fold(f64::INFINITY, f64::min)
        };
        let mut order: Vec<usize> = by_cand.keys().copied().collect();
        order.sort_by(|&a, &b| {
            key(a)
                .total_cmp(&key(b))
                .then(cands[a].obj.cmp(&cands[b].obj))
        });
        let strands_of = |ci: usize| -> BTreeSet<usize> {
            entries
                .iter()
                .filter(|e| by_cand[&ci].contains(&e.placed))
                .filter_map(strand_of)
                .collect()
        };
        let mut winners: Vec<usize> = Vec::new();
        let mut removed: BTreeSet<usize> = BTreeSet::new();
        for &ci in &order {
            let mine = strands_of(ci);
            let clash = winners.iter().any(|&w| {
                haversine_m(cands[w].pos, cands[ci].pos) <= AUTOHOF_SITE_DEDUPE_M
                    && strands_of(w).intersection(&mine).next().is_some()
            });
            if clash {
                removed.insert(ci);
                counts.site_duplicates += 1;
                dropped.push(Dropped {
                    cand: ci,
                    cluster: placed[by_cand[&ci][0]].cluster,
                    why: format!(
                        "duplicate site: another Autohof within {AUTOHOF_SITE_DEDUPE_M:.0} m on the same strand has the smaller detourKm"
                    ),
                });
            } else {
                winners.push(ci);
            }
        }
        if !removed.is_empty() {
            let mut remap: Vec<Option<usize>> = Vec::with_capacity(placed.len());
            let mut next = 0;
            for p in &placed {
                if removed.contains(&p.cand) {
                    remap.push(None);
                } else {
                    remap.push(Some(next));
                    next += 1;
                }
            }
            let mut i = 0;
            placed.retain(|_| {
                i += 1;
                remap[i - 1].is_some()
            });
            entries.retain_mut(|e| match remap[e.placed] {
                Some(n) => {
                    e.placed = n;
                    true
                }
                None => false,
            });
        }
    }
    entries.sort_by(|a, b| {
        a.strand_id
            .as_bytes()
            .cmp(b.strand_id.as_bytes())
            .then(a.km.total_cmp(&b.km))
            .then(a.id.as_bytes().cmp(b.id.as_bytes()))
    });
    counts.entries = entries.len();
    counts.distinct = placed
        .iter()
        .map(|p| cands[p.cand].obj)
        .collect::<BTreeSet<_>>()
        .len();
    AutohofResult {
        clusters,
        cands,
        placed,
        entries,
        additional,
        unplaced,
        dropped,
        counts,
    }
}

pub fn cluster_label(c: &Cluster) -> String {
    format!("{} {}", c.motorway, c.exit_text())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brands;
    use crate::facilities::ObjType;
    use crate::geo::local::cumulative_km;

    fn tags(kv: &[(&str, &str)]) -> Tags {
        Tags(
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    /// Straight strand along a meridian from `lat0` for `n` km (about, 0.009 deg per km).
    fn strand(id: &str, ref_: &str, lon: f64, lat0: f64, dlat: f64) -> Strand {
        let pts: Vec<LatLon> = (0..=20)
            .map(|i| LatLon::new(lat0 + dlat * i as f64, lon))
            .collect();
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

    fn junction(
        node_id: i64,
        strand: &Strand,
        lat: f64,
        jref: &str,
        name: Option<&str>,
    ) -> Junction {
        Junction {
            node_id,
            pos: LatLon::new(lat, strand.points[0].lon),
            strand_id: strand.id.clone(),
            km: 5.0,
            ref_: Some(jref.into()),
            name: name.map(String::from),
        }
    }

    fn brand_list() -> Vec<Brand> {
        brands::parse(
            r#"[{"id":"aral","displayName":"Aral","category":"fuel","match":{"brand":["Aral"],"name":["Aral"]},"simpleIconsSlug":null},
                {"id":"burger-king","displayName":"BK","category":"food","match":{"brand":["Burger King"],"name":["Burger King"]},"simpleIconsSlug":null},
                {"id":"ionity","displayName":"Ionity","category":"charging","match":{"brand":["Ionity"],"name":["Ionity"]},"simpleIconsSlug":null}]"#,
            "t",
        )
        .unwrap()
    }

    fn poi<'a>(id: i64, lat: f64, lon: f64, t: &'a Tags) -> Poi<'a> {
        Poi {
            obj: ObjRef {
                typ: ObjType::Node,
                id,
            },
            pos: LatLon::new(lat, lon),
            tags: t,
        }
    }

    /// Two carriageways 30 m apart, junction nodes on both at lat 52.0.
    struct World {
        strands: Vec<Strand>,
        junctions: Vec<Junction>,
    }

    fn world() -> World {
        let a = strand("A1-N-0", "A 1", 10.0, 51.9, 0.009);
        let b = strand("A1-S-0", "A 1", 10.0004, 51.9, 0.009);
        let junctions = vec![
            junction(1, &a, 52.0, "10", Some("Testhausen")),
            junction(2, &b, 52.0002, "10", Some("Testhausen")),
        ];
        World {
            strands: vec![a, b],
            junctions,
        }
    }

    fn run(
        w: &World,
        pois: &[Poi<'_>],
        deferred: &[Deferred],
        assigned: &[usize],
    ) -> AutohofResult {
        run_p(w, pois, deferred, assigned, &[])
    }

    fn run_p(
        w: &World,
        pois: &[Poi<'_>],
        deferred: &[Deferred],
        assigned: &[usize],
        parkings: &[LatLon],
    ) -> AutohofResult {
        let b = brand_list();
        let m = Matcher::new(&b);
        let assigned: HashSet<usize> = assigned.iter().copied().collect();
        let none = HashSet::new();
        build(&AutohofInput {
            strands: &w.strands,
            junctions: &w.junctions,
            pois,
            assigned: &assigned,
            invariant_excluded: &none,
            rast_sources: &[],
            rastanlagen: &[],
            deferred,
            matcher: &m,
            brands: &b,
            truck_parkings: parkings,
        })
    }

    fn deferred_node(id: i64, lat: f64, lon: f64, kv: &[(&str, &str)]) -> Deferred {
        let t = tags(kv);
        Deferred {
            obj: ObjRef {
                typ: ObjType::Node,
                id,
            },
            name: t.get("name").unwrap_or("-").to_string(),
            reason: "test".into(),
            strand: None,
            source: Source {
                obj: ObjRef {
                    typ: ObjType::Node,
                    id,
                },
                tags: t,
                rings: Vec::new(),
                centroid: LatLon::new(lat, lon),
            },
        }
    }

    #[test]
    fn clusters_merge_close_nodes_with_same_ref_and_name() {
        let w = world();
        let c = build_clusters(&w.strands, &w.junctions);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].nodes.len(), 2);
        assert_eq!(c[0].exit_text(), "10 Testhausen");
    }

    #[test]
    fn clusters_split_by_name_distance_and_motorway() {
        let mut w = world();
        // Different name -> own cluster; same name but 3.3 km away -> own cluster.
        w.junctions
            .push(junction(3, &w.strands[0], 52.03, "10", Some("Testhausen")));
        w.junctions
            .push(junction(4, &w.strands[0], 52.01, "11", Some("Other")));
        let c = build_clusters(&w.strands, &w.junctions);
        assert_eq!(c.len(), 3);
        // Same junction on another motorway ref is its own cluster as well.
        let mut w2 = world();
        w2.strands[1].ref_ = "A 7".into();
        assert_eq!(build_clusters(&w2.strands, &w2.junctions).len(), 2);
    }

    #[test]
    fn keyword_qualifies_and_gives_two_entries_for_two_carriageways() {
        let w = world();
        let t = tags(&[
            ("amenity", "fuel"),
            ("name", "Foo Autohof-Tankstelle"),
            ("brand", "Aral"),
        ]);
        let pois = vec![poi(10, 52.005, 10.01, &t)];
        let r = run(&w, &pois, &[], &[]);
        assert_eq!(r.counts.qualified, 1);
        assert!(r.counts.q_keyword == 1 && r.counts.q_gastro == 0);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.counts.distinct, 1);
        let ids: Vec<&str> = r.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["osm-node-10@A1-N-0", "osm-node-10@A1-S-0"]);
        assert!(r.entries.iter().all(|e| e.kind == "autohof"));
        assert!(r
            .entries
            .iter()
            .all(|e| e.exit.as_deref() == Some("10 Testhausen") && e.detour_km > 0.0));
        assert_eq!(r.entries[0].brands, ["aral"]);
    }

    /// Fuel station + Burger King next to it at `lat`, ids `id` and `id + 1`.
    fn fuel_with_bk<'a>(id: i64, lat: f64, fuel: &'a Tags, bk: &'a Tags) -> Vec<Poi<'a>> {
        vec![
            poi(id, lat, 10.01, fuel),
            poi(id + 1, lat + 0.001, 10.01, bk),
        ]
    }

    #[test]
    fn rule3_needs_gastro_within_150m_and_truck_parking_within_300m() {
        let w = world();
        let fuel = tags(&[("amenity", "fuel")]);
        let bk = tags(&[("amenity", "fast_food"), ("brand", "Burger King")]);
        let far = tags(&[("amenity", "fast_food")]);
        // 0.001 deg lat = 111 m. Fuel 1: BK at 111 m, parking at 22 m -> Autohof.
        let mut pois = fuel_with_bk(1, 52.005, &fuel, &bk);
        // Fuel 3: BK at 222 m (too far), parking near.
        pois.push(poi(3, 52.0, 10.02, &fuel));
        pois.push(poi(4, 52.002, 10.02, &far));
        let parkings = [LatLon::new(52.0052, 10.01), LatLon::new(52.0002, 10.02)];
        let r = run_p(&w, &pois, &[], &[], &parkings);
        assert_eq!(r.counts.candidates, 2);
        assert_eq!(r.counts.q_gastro, 1);
        assert_eq!(r.counts.qualified, 1);
        assert_eq!(r.entries[0].id, "osm-node-1@A1-N-0");
        assert_eq!(r.entries[0].brands, ["burger-king"]);
        // Without any truck parking the same fuel station is not an Autohof.
        let r = run_p(&w, &pois, &[], &[], &[]);
        assert_eq!(r.counts.qualified, 0);
        assert!(r.entries.is_empty());
        // Parking at 311 m (beyond 300 m) does not count either.
        let r = run_p(&w, &pois, &[], &[], &[LatLon::new(52.0078, 10.01)]);
        assert_eq!(r.counts.qualified, 0);
    }

    #[test]
    fn rule3_requires_junction_within_1km_but_keyword_does_not() {
        let w = world();
        let fuel = tags(&[("amenity", "fuel")]);
        let kw = tags(&[("amenity", "fuel"), ("name", "Autohof K")]);
        let bk = tags(&[("amenity", "fast_food")]);
        // Fuel at 52.012 is about 1.33 km from the junction (52.0 / 52.0002).
        let mut pois = fuel_with_bk(1, 52.012, &fuel, &bk);
        pois.push(poi(5, 52.0121, 10.5, &kw)); // other place, > 2 km away: no candidate
        let parkings = [LatLon::new(52.0121, 10.01)];
        let r = run_p(&w, &pois, &[], &[], &parkings);
        assert_eq!(r.counts.q_gastro, 1);
        assert!(r.entries.is_empty(), "rule 3 beyond 1 km must not emit");
        // A keyword candidate at the same distance is fine.
        let pois = vec![poi(7, 52.012, 10.01, &kw)];
        let r = run(&w, &pois, &[], &[]);
        assert_eq!(r.entries.len(), 2);
    }

    #[test]
    fn stronger_rule_beats_nearer_candidate() {
        let w = world();
        let near = tags(&[("amenity", "fuel")]);
        let bk = tags(&[("amenity", "fast_food")]);
        let kw = tags(&[("amenity", "fuel"), ("name", "Autohof Weit")]);
        // Rule 3 station at ~220 m, keyword station at ~780 m, deferred at ~1.3 km.
        let mut pois = fuel_with_bk(1, 52.002, &near, &bk);
        pois.push(poi(10, 52.007, 10.0, &kw));
        let parkings = [LatLon::new(52.002, 10.0105)];
        let r = run_p(&w, &pois, &[], &[], &parkings);
        assert_eq!(r.counts.qualified, 2);
        assert_eq!(r.entries[0].name.as_deref(), Some("Autohof Weit"));
        assert_eq!(r.additional.len(), 1);
        assert_eq!(r.cands[r.additional[0].cand].obj.id, 1);
        // A deferred services object beats both.
        let d = deferred_node(99, 52.012, 10.0, &[("name", "Rasthof Weit")]);
        let r = run_p(&w, &pois, &[d], &[], &parkings);
        assert_eq!(r.entries[0].id, "osm-node-99@A1-N-0");
        assert_eq!(r.additional.len(), 2);
    }

    #[test]
    fn rest_area_clusters_only_serve_deferred_services_objects() {
        let a = strand("A1-N-0", "A 1", 10.0, 51.9, 0.009);
        let junctions = vec![
            junction(1, &a, 52.0, "20", Some("Raststätte Harz")),
            junction(2, &a, 52.1, "21", Some("RASTPLATZ Nord")),
        ];
        let w = World {
            strands: vec![a],
            junctions,
        };
        assert!(build_clusters(&w.strands, &w.junctions)
            .iter()
            .all(Cluster::is_rest_area));
        let kw = tags(&[("amenity", "fuel"), ("name", "Autohof K")]);
        let fuel = tags(&[("amenity", "fuel")]);
        let bk = tags(&[("amenity", "fast_food")]);
        let mut pois = vec![poi(1, 52.002, 10.01, &kw)];
        pois.extend(fuel_with_bk(5, 52.102, &fuel, &bk));
        let parkings = [LatLon::new(52.102, 10.0105)];
        let r = run_p(&w, &pois, &[], &[], &parkings);
        assert!(
            r.entries.is_empty(),
            "rule 2 and 3 are skipped in Rast* clusters"
        );
        // A deferred services object may use the Rast* cluster.
        let d = deferred_node(50, 52.003, 10.0, &[("name", "Esso A 7 Raststaette")]);
        let r = run_p(&w, &pois, &[d], &[], &parkings);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].id, "osm-node-50@A1-N-0");
        assert_eq!(r.entries[0].exit.as_deref(), Some("20 Raststätte Harz"));
    }

    #[test]
    fn non_rast_names_are_regular_junctions() {
        let w = world();
        assert!(!build_clusters(&w.strands, &w.junctions)[0].is_rest_area());
        let c = Cluster {
            motorway: "A 1".into(),
            exit: String::new(),
            jname: Some("Rastede".into()),
            nodes: vec![],
        };
        // Rastede is a town, not a rest area.
        assert!(!c.is_rest_area());
        for n in [
            "Rastplatz X",
            "Raststätte Y",
            "Rasthof Z",
            "Rastanlage W",
            "rasthof z",
        ] {
            let c = Cluster {
                jname: Some(n.into()),
                ..c.clone()
            };
            assert!(c.is_rest_area(), "{n}");
        }
    }

    #[test]
    fn assigned_and_distant_fuel_are_no_candidates() {
        let w = world();
        let t = tags(&[("amenity", "fuel"), ("name", "Autohof X")]);
        let pois = vec![poi(1, 52.005, 10.01, &t), poi(2, 52.05, 10.0, &t)];
        let r = run(&w, &pois, &[], &[0]);
        // #1 belongs to a Rastanlage, #2 is > 2 km from the junction.
        assert_eq!(r.counts.fuel_assigned, 1);
        assert_eq!(r.counts.candidates, 0);
        assert!(r.entries.is_empty());
    }

    #[test]
    fn nearest_autohof_per_cluster_others_are_additional() {
        let w = world();
        let t1 = tags(&[("amenity", "fuel"), ("name", "Autohof Near")]);
        let t2 = tags(&[("amenity", "fuel"), ("name", "Autohof Far")]);
        let pois = vec![poi(1, 52.012, 10.0, &t2), poi(2, 52.004, 10.0, &t1)];
        let r = run(&w, &pois, &[], &[]);
        assert_eq!(r.counts.qualified, 2);
        assert_eq!(r.counts.distinct, 1);
        assert_eq!(r.entries[0].name.as_deref(), Some("Autohof Near"));
        assert_eq!(r.additional.len(), 1);
        assert_eq!(r.cands[r.additional[0].cand].obj.id, 1);
    }

    #[test]
    fn candidate_is_kept_for_the_nearest_cluster_only() {
        let mut w = world();
        // Second cluster ("Other", ref 11) 2.2 km north on strand A.
        w.junctions
            .push(junction(5, &w.strands[0], 52.02, "11", Some("Other")));
        // The Autohof is between: 0.9 km from the first (52.0), 1.3 km from the second.
        let t = tags(&[("amenity", "fuel"), ("name", "Autohof Mitte")]);
        let pois = vec![poi(1, 52.0075, 10.0, &t)];
        let r = run(&w, &pois, &[], &[]);
        assert_eq!(r.clusters.len(), 2);
        assert_eq!(r.entries.len(), 2); // both carriageways of cluster 1 only
        assert!(r
            .entries
            .iter()
            .all(|e| e.exit.as_deref() == Some("10 Testhausen")));
        assert_eq!(r.additional.len(), 1);
        assert!(r.additional[0].why.contains("nearer exit"));
    }

    /// World with a second exit on the south strand only (name differs, 1.3 km north of the
    /// first exit's node there), as at Autohof Frechen.
    fn two_exit_world() -> World {
        let mut w = world();
        w.junctions.push(junction(
            6,
            &w.strands[1],
            52.0140,
            "11",
            Some("Gegenrichtung"),
        ));
        w.junctions.remove(1); // no node of exit 10 on the south strand
        w
    }

    #[test]
    fn nearest_site_per_strand_gives_both_directions() {
        let w = two_exit_world();
        let t = tags(&[("amenity", "fuel"), ("name", "Autohof Frechen")]);
        // 0.9 km north of exit 10 (north strand), 0.5 km south of exit 11 (south strand).
        let pois = vec![poi(1, 52.0090, 10.0002, &t)];
        let r = run(&w, &pois, &[], &[]);
        assert_eq!(r.clusters.len(), 2);
        assert_eq!(r.counts.distinct, 1);
        assert_eq!(r.entries.len(), 2);
        let mut exits: Vec<(&str, &str)> = r
            .entries
            .iter()
            .map(|e| (e.strand_id.as_str(), e.exit.as_deref().unwrap()))
            .collect();
        exits.sort();
        assert_eq!(
            exits,
            [("A1-N-0", "10 Testhausen"), ("A1-S-0", "11 Gegenrichtung")]
        );
    }

    #[test]
    fn cluster_key_ignores_order_of_semicolon_lists() {
        let a = strand("A1-N-0", "A 1", 10.0, 51.9, 0.009);
        let b = strand("A1-S-0", "A 1", 10.0004, 51.9, 0.009);
        let junctions = vec![
            junction(1, &a, 52.0, "24;23", Some("Dreieck D;D-Hasport")),
            junction(2, &b, 52.0002, "23; 24", Some("D-Hasport;Dreieck D")),
        ];
        let c = build_clusters(&[a, b], &junctions);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].nodes.len(), 2);
        assert!(!c[0].exit.contains(';'));
        assert_eq!(c[0].exit, "24 Dreieck D / 23 D-Hasport");
    }

    #[test]
    fn rule2_accepts_truckstop_and_truck_stop() {
        for name in [
            "TotalEnergies Truckstop Oyten",
            "Truck Stop Düren",
            "Autohof X",
        ] {
            let t = tags(&[("amenity", "fuel"), ("name", name)]);
            assert!(has_autohof_keyword(&t), "{name}");
        }
        for name in ["Truck Wash", "Maxi Tank", "Shell"] {
            let t = tags(&[("amenity", "fuel"), ("name", name)]);
            assert!(!has_autohof_keyword(&t), "{name}");
        }
    }

    /// Single strand with two exits 0.6 km apart (52.0 "Eins" and 52.0055 "Zwei").
    fn two_exit_one_strand() -> World {
        let a = strand("A1-N-0", "A 1", 10.0, 51.9, 0.009);
        let junctions = vec![
            junction(1, &a, 52.0, "10", Some("Eins")),
            junction(2, &a, 52.0055, "11", Some("Zwei")),
        ];
        World {
            strands: vec![a],
            junctions,
        }
    }

    #[test]
    fn site_dedupe_keeps_smallest_detour() {
        let w = two_exit_one_strand();
        let t1 = tags(&[("amenity", "fuel"), ("name", "Autohof Fern")]);
        let t2 = tags(&[("amenity", "fuel"), ("name", "Autohof Nah")]);
        let t3 = tags(&[("amenity", "fuel"), ("name", "Autohof Weit")]);
        // 100 m apart: #1 is 300 m from exit 10 (detour 0.4), #2 is 211 m from exit 11 (0.3).
        let pois = vec![poi(1, 52.0027, 10.0, &t1), poi(2, 52.0036, 10.0, &t2)];
        let r = run(&w, &pois, &[], &[]);
        assert_eq!(r.clusters.len(), 2);
        assert_eq!(r.counts.site_duplicates, 1);
        assert_eq!(r.counts.distinct, 1);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].id, "osm-node-2@A1-N-0");
        assert_eq!(r.dropped.len(), 1);
        assert!(r.dropped[0].why.contains("duplicate site"));
        // 200 m apart both stay (#3 is 211 m from exit 11 as well, but 200 m from #1... use
        // a station 250 m south of #2 instead).
        let pois = vec![poi(1, 52.0027, 10.0, &t1), poi(3, 52.0045, 10.0, &t3)];
        let r = run(&w, &pois, &[], &[]);
        assert_eq!(r.counts.site_duplicates, 0);
        assert_eq!(r.counts.distinct, 2);
    }

    #[test]
    fn site_dedupe_ties_go_to_the_smaller_object_id() {
        let w = two_exit_one_strand();
        let t1 = tags(&[("amenity", "fuel"), ("name", "Autohof Zwanzig")]);
        let t2 = tags(&[("amenity", "fuel"), ("name", "Autohof Zehn")]);
        // 11 m apart, both 300 m from "their" exit: equal detour 0.4.
        let pois = vec![poi(10, 52.0028, 10.0, &t2), poi(20, 52.0027, 10.0, &t1)];
        let r = run(&w, &pois, &[], &[]);
        assert_eq!(r.counts.site_duplicates, 1);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].id, "osm-node-10@A1-N-0");
    }

    #[test]
    fn autohof_at_a_rastanlage_is_dropped() {
        let w = world();
        let t = tags(&[("amenity", "fuel"), ("name", "Autohof R")]);
        let pois = vec![poi(1, 52.005, 10.0, &t)];
        let mk_rast = |lat: f64| {
            (
                ObjRef {
                    typ: ObjType::Way,
                    id: 77,
                },
                LatLon::new(lat, 10.0),
            )
        };
        let b = brand_list();
        let m = Matcher::new(&b);
        let none = HashSet::new();
        let go = |rast: &[(ObjRef, LatLon)]| {
            build(&AutohofInput {
                strands: &w.strands,
                junctions: &w.junctions,
                pois: &pois,
                assigned: &none,
                invariant_excluded: &none,
                rast_sources: &[],
                rastanlagen: rast,
                deferred: &[],
                matcher: &m,
                brands: &b,
                truck_parkings: &[],
            })
        };
        let r = go(&[mk_rast(52.0059)]); // 100 m
        assert!(r.entries.is_empty());
        assert_eq!(r.counts.at_rastanlage, 1);
        // 200 m away it stays.
        let r = go(&[mk_rast(52.0068)]);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.counts.at_rastanlage, 0);
    }

    #[test]
    fn deferred_services_qualify_automatically_and_absorb_fuel() {
        let w = world();
        let d = deferred_node(
            100,
            52.004,
            10.0,
            &[
                ("highway", "services"),
                ("name", "Truck Stop"),
                ("brand", "Aral"),
            ],
        );
        let fuel = tags(&[("amenity", "fuel"), ("name", "Tankstelle")]);
        // fuel 100 m from the services node, merged; second fuel far away, no gastro -> nothing.
        let far = tags(&[("amenity", "fuel")]);
        let pois = vec![poi(1, 52.0049, 10.0, &fuel), poi(2, 52.009, 10.0, &far)];
        let r = run(&w, &pois, &[d], &[]);
        assert_eq!(r.counts.deferred_candidates, 1);
        assert_eq!(r.counts.fuel_merged_into_services, 1);
        assert_eq!(r.counts.q_deferred, 1);
        assert_eq!(r.entries.len(), 2);
        // ID and name from the services object, position from the fuel station.
        assert_eq!(r.entries[0].id, "osm-node-100@A1-N-0");
        assert_eq!(r.entries[0].name.as_deref(), Some("Truck Stop"));
        assert!((r.entries[0].lat - 52.0049).abs() < 1e-9);
        assert_eq!(r.entries[0].brands, ["aral"]);
    }

    #[test]
    fn deferred_far_from_every_junction_is_unplaced() {
        let w = world();
        let d = deferred_node(
            100,
            52.1,
            10.0,
            &[("highway", "services"), ("name", "Maxi")],
        );
        let r = run(&w, &[], &[d], &[]);
        assert_eq!(r.unplaced, vec![0]);
        assert_eq!(r.counts.candidates, 0);
        assert!(r.entries.is_empty());
    }

    #[test]
    fn detour_rounding() {
        assert_eq!(detour_km(1000.0), 1.3);
        assert_eq!(detour_km(1100.0), 1.4); // 1.43 -> 1.4
        assert_eq!(detour_km(1150.0), 1.5); // 1.495 -> 1.5
        assert_eq!(detour_km(10.0), 0.1); // never 0
    }

    #[test]
    fn amenities_from_circle() {
        let w = world();
        let fuel = tags(&[("amenity", "fuel"), ("name", "Autohof Y")]);
        let shower = tags(&[("amenity", "shower")]);
        let cafe = tags(&[("amenity", "cafe"), ("opening_hours", "24/7")]);
        let chg = tags(&[("amenity", "charging_station"), ("operator", "Ionity")]);
        let pois = vec![
            poi(1, 52.005, 10.0, &fuel),
            poi(2, 52.0051, 10.0, &shower),
            poi(3, 52.0052, 10.0, &cafe),
            poi(4, 52.0053, 10.0, &chg),
        ];
        let r = run(&w, &pois, &[], &[]);
        let e = &r.entries[0];
        assert!(e.shower && e.open24h && e.fast_charger);
        assert_eq!(e.brands, ["ionity"]);
    }

    #[test]
    fn shared_section_clusters_keep_one_entry_per_strand() {
        // The same junction node on two strands with different refs (A 1;A 3).
        let a = strand("A1-N-0", "A 1", 10.0, 51.9, 0.009);
        let b = strand("A3-N-0", "A 3", 10.0, 51.9, 0.009);
        let junctions = vec![
            junction(1, &a, 52.0, "10", Some("Testhausen")),
            junction(1, &b, 52.0, "10", Some("Testhausen")),
        ];
        let w = World {
            strands: vec![a, b],
            junctions,
        };
        let t = tags(&[("amenity", "fuel"), ("name", "Autohof Z")]);
        let pois = vec![poi(1, 52.004, 10.0, &t)];
        let r = run(&w, &pois, &[], &[]);
        assert_eq!(r.clusters.len(), 2);
        assert_eq!(r.entries.len(), 2);
        assert!(r.additional.is_empty());
    }
}
