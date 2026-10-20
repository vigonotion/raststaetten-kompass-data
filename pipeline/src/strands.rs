//! Strand construction (heuristics §2–3): chaining, direction, simplification, junctions.

use crate::geo::distance::{angle_diff_deg, bearing_deg, haversine_m, towards};
use crate::geo::local::{cumulative_km, project_onto_polyline};
use crate::geo::polyline;
use crate::geo::simplify::douglas_peucker;
use crate::geo::{round_to, LatLon};
use crate::osm::model::{Model, Oneway};
use crate::params::{MIN_STRAND_LENGTH_KM, SIMPLIFY_TOLERANCE_M, UTURN_GUARD_DEG};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug)]
pub struct Strand {
    pub id: String,
    pub ref_: String,
    pub towards: &'static str,
    pub polyline: String,
    /// Decoded (rounded) points, as seen by readers.
    pub points: Vec<LatLon>,
    pub cum_km: Vec<f64>,
    pub length_km: f64,
    pub first_way_id: i64,
    #[allow(dead_code)] // kept for debugging and tests
    pub way_ids: Vec<i64>,
}

#[derive(Clone, Debug)]
pub struct Junction {
    /// OSM node ID of the `motorway_junction` node (identifies shared-section duplicates).
    #[allow(dead_code)]
    pub node_id: i64,
    /// Position of the junction node (not the projection onto the strand).
    pub pos: LatLon,
    pub strand_id: String,
    pub km: f64,
    pub ref_: Option<String>,
    pub name: Option<String>,
}

/// Splits an OSM `;` list into trimmed, non-empty values.
pub fn split_list(s: &str) -> Vec<&str> {
    s.split(';')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect()
}

/// Cluster key part (D-33): split at `;`, trim, sort, re-join, so `A;B` and `B;A` are equal.
pub fn normalize_list(s: &str) -> String {
    let mut v = split_list(s);
    v.sort_unstable();
    v.join(";")
}

/// Display of a `ref` or `name` list (schema §2.3): values joined with `" / "`.
pub fn display_list(s: &str) -> String {
    split_list(s).join(" / ")
}

/// Display text of a junction (schema §2.3): `"<ref> <name>"` or whichever exists. Lists are
/// paired value by value (`24;23` + `A;B` -> `"24 A / 23 B"`); with unequal lengths refs and
/// names are each joined with `" / "`.
pub fn display_exit(ref_: Option<&str>, name: Option<&str>) -> String {
    let r = ref_.map(split_list).unwrap_or_default();
    let n = name.map(split_list).unwrap_or_default();
    match (r.is_empty(), n.is_empty()) {
        (true, true) => String::new(),
        (false, true) => r.join(" / "),
        (true, false) => n.join(" / "),
        (false, false) if r.len() == n.len() => r
            .iter()
            .zip(&n)
            .map(|(a, b)| format!("{a} {b}"))
            .collect::<Vec<_>>()
            .join(" / "),
        (false, false) => format!("{} {}", r.join(" / "), n.join(" / ")),
    }
}

#[derive(Clone, Debug)]
pub struct Remnant {
    pub ref_: String,
    pub length_km: f64,
    pub way_ids: Vec<i64>,
}

pub struct StrandSet {
    pub strands: Vec<Strand>,
    pub junctions: Vec<Junction>,
    pub remnants: Vec<Remnant>,
}

/// A directed edge of the chaining graph (one way in one direction).
struct Edge {
    way_id: i64,
    from: i64,
    to: i64,
    pts: Vec<LatLon>,
    nodes: Vec<i64>,
    length_m: f64,
    first_bearing: f64,
    last_bearing: f64,
}

struct Chain {
    pts: Vec<LatLon>,
    way_ids: Vec<i64>,
    nodes: Vec<i64>,
}

fn build_edge(way_id: i64, nodes: &[i64], model: &Model) -> Option<Edge> {
    let mut ids = Vec::with_capacity(nodes.len());
    let mut pts: Vec<LatLon> = Vec::with_capacity(nodes.len());
    for id in nodes {
        if let Some(p) = model.coords.get(id) {
            if pts.last() == Some(p) {
                continue; // consecutive duplicate position
            }
            ids.push(*id);
            pts.push(*p);
        }
    }
    if pts.len() < 2 {
        return None;
    }
    let length_m = pts.windows(2).map(|w| haversine_m(w[0], w[1])).sum();
    let first_bearing = bearing_deg(pts[0], pts[1]);
    let n = pts.len();
    let last_bearing = bearing_deg(pts[n - 2], pts[n - 1]);
    Some(Edge {
        way_id,
        from: ids[0],
        to: ids[n - 1],
        pts,
        nodes: ids,
        length_m,
        first_bearing,
        last_bearing,
    })
}

fn reversed(e: &Edge) -> Edge {
    let mut pts = e.pts.clone();
    pts.reverse();
    let mut nodes = e.nodes.clone();
    nodes.reverse();
    Edge {
        way_id: e.way_id,
        from: e.to,
        to: e.from,
        pts,
        nodes,
        length_m: e.length_m,
        first_bearing: (e.last_bearing + 180.0).rem_euclid(360.0),
        last_bearing: (e.first_bearing + 180.0).rem_euclid(360.0),
    }
}

/// Chaining state for one A number.
struct Chainer {
    edges: Vec<Edge>,
    succ: Vec<Vec<usize>>,
    pred: Vec<Vec<usize>>,
    used: Vec<bool>,
    memo: Vec<Option<f64>>,
    on_stack: Vec<bool>,
}

impl Chainer {
    fn new(edges: Vec<Edge>) -> Self {
        let n = edges.len();
        let mut by_from: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, e) in edges.iter().enumerate() {
            by_from.entry(e.from).or_default().push(i);
        }
        let mut succ = vec![Vec::new(); n];
        let mut pred = vec![Vec::new(); n];
        for (i, e) in edges.iter().enumerate() {
            if let Some(next) = by_from.get(&e.to) {
                for &j in next {
                    // U-turn guard: reversing the bearing by more than 150 degrees is not a continuation.
                    if angle_diff_deg(e.last_bearing, edges[j].first_bearing) <= UTURN_GUARD_DEG {
                        succ[i].push(j);
                        pred[j].push(i);
                    }
                }
            }
        }
        Chainer {
            edges,
            succ,
            pred,
            used: vec![false; n],
            memo: vec![None; n],
            on_stack: vec![false; n],
        }
    }

    /// Length of the longest continuation starting with edge `e` over unused edges.
    /// Iterative memoized DFS (explicit stack, no recursion) with the cycle guard: an edge that
    /// is on the current DFS stack counts as a non-existent successor.
    fn down(&mut self, e: usize) -> f64 {
        if let Some(v) = self.memo[e] {
            return v;
        }
        // Frame: (edge, index of the next successor to visit, best continuation so far).
        let mut stack: Vec<(usize, usize, f64)> = vec![(e, 0, 0.0)];
        self.on_stack[e] = true;
        let mut result = 0.0;
        while let Some(&mut (cur, ref mut k, ref mut best)) = stack.last_mut() {
            if *k < self.succ[cur].len() {
                let f = self.succ[cur][*k];
                *k += 1;
                if self.used[f] || self.on_stack[f] {
                    continue;
                }
                if let Some(d) = self.memo[f] {
                    if d > *best {
                        *best = d;
                    }
                } else {
                    self.on_stack[f] = true;
                    stack.push((f, 0, 0.0));
                }
            } else {
                let v = self.edges[cur].length_m + *best;
                self.on_stack[cur] = false;
                self.memo[cur] = Some(v);
                stack.pop();
                result = v;
                if let Some((_, _, parent_best)) = stack.last_mut() {
                    if v > *parent_best {
                        *parent_best = v;
                    }
                }
            }
        }
        result
    }

    fn next_chain(&mut self) -> Option<Chain> {
        if self.used.iter().all(|u| *u) {
            return None;
        }
        self.memo.iter_mut().for_each(|m| *m = None);
        let n = self.edges.len();
        let mut start: Option<(usize, f64)> = None;
        for pass_sources_only in [true, false] {
            for e in 0..n {
                if self.used[e] {
                    continue;
                }
                if pass_sources_only && self.pred[e].iter().any(|p| !self.used[*p]) {
                    continue;
                }
                let d = self.down(e);
                if start.is_none_or(|(_, bd)| d > bd) {
                    start = Some((e, d));
                }
            }
            if start.is_some() {
                break;
            }
        }
        let (mut cur, _) = start?;
        let mut chain = Chain {
            pts: Vec::new(),
            way_ids: Vec::new(),
            nodes: Vec::new(),
        };
        loop {
            self.used[cur] = true;
            let e = &self.edges[cur];
            if chain.pts.is_empty() {
                chain.pts.extend_from_slice(&e.pts);
                chain.nodes.extend_from_slice(&e.nodes);
            } else {
                chain.pts.extend_from_slice(&e.pts[1..]);
                chain.nodes.extend_from_slice(&e.nodes[1..]);
            }
            chain.way_ids.push(e.way_id);
            let mut next: Option<(usize, f64)> = None;
            for k in 0..self.succ[cur].len() {
                let f = self.succ[cur][k];
                if self.used[f] {
                    continue;
                }
                let d = self.down(f);
                if next.is_none_or(|(_, bd)| d > bd) {
                    next = Some((f, d));
                }
            }
            match next {
                Some((f, _)) => cur = f,
                None => break,
            }
        }
        Some(chain)
    }
}

/// Builds all strands, junctions and remnants from the model.
pub fn build(model: &Model) -> StrandSet {
    // Ways per A number, ordered by way id (BTreeMap) for determinism.
    let mut per_ref: BTreeMap<u32, Vec<i64>> = BTreeMap::new();
    for (id, w) in &model.motorway_ways {
        for n in &w.a_numbers {
            per_ref.entry(*n).or_default().push(*id);
        }
    }

    struct Pending {
        a: u32,
        towards: &'static str,
        polyline: String,
        points: Vec<LatLon>,
        cum_km: Vec<f64>,
        length_km: f64,
        first_way_id: i64,
        way_ids: Vec<i64>,
        junction_nodes: Vec<i64>,
    }
    let mut pending: Vec<Pending> = Vec::new();
    let mut remnants = Vec::new();

    for (a, way_ids) in &per_ref {
        let mut edges = Vec::new();
        for id in way_ids {
            let w = &model.motorway_ways[id];
            let Some(fwd) = build_edge(*id, &w.nodes, model) else {
                continue;
            };
            match w.oneway {
                Oneway::Forward => edges.push(fwd),
                Oneway::Reverse => edges.push(reversed(&fwd)),
                Oneway::Both => {
                    let rev = reversed(&fwd);
                    edges.push(fwd);
                    edges.push(rev);
                }
            }
        }
        let mut chainer = Chainer::new(edges);
        while let Some(chain) = chainer.next_chain() {
            let raw_len_km = chain
                .pts
                .windows(2)
                .map(|w| haversine_m(w[0], w[1]))
                .sum::<f64>()
                / 1000.0;
            if raw_len_km <= MIN_STRAND_LENGTH_KM {
                remnants.push(Remnant {
                    ref_: format!("A {a}"),
                    length_km: round_to(raw_len_km, 3),
                    way_ids: chain.way_ids,
                });
                continue;
            }
            let simplified = douglas_peucker(&chain.pts, SIMPLIFY_TOLERANCE_M);
            let encoded = polyline::encode(&simplified);
            let points = polyline::decode(&encoded).expect("own polyline decodes");
            let cum_km = cumulative_km(&points);
            let length_km = round_to(*cum_km.last().unwrap_or(&0.0), 3);
            let bearing = bearing_deg(points[0], points[points.len() - 1]);
            let mut jn: Vec<i64> = chain
                .nodes
                .iter()
                .copied()
                .filter(|n| model.junction_nodes.contains_key(n))
                .collect();
            jn.sort_unstable();
            jn.dedup();
            pending.push(Pending {
                a: *a,
                towards: towards(bearing),
                polyline: encoded,
                points,
                cum_km,
                length_km,
                first_way_id: chain.way_ids[0],
                way_ids: chain.way_ids,
                junction_nodes: jn,
            });
        }
    }

    // IDs: numbered per (ref, towards) by descending length, ties by first way id.
    pending.sort_by(|x, y| {
        (x.a, x.towards)
            .cmp(&(y.a, y.towards))
            .then(y.length_km.total_cmp(&x.length_km))
            .then(x.first_way_id.cmp(&y.first_way_id))
    });
    let mut strands = Vec::new();
    let mut junctions = Vec::new();
    let mut counter: HashMap<(u32, &'static str), usize> = HashMap::new();
    for p in pending {
        let n = counter.entry((p.a, p.towards)).or_insert(0);
        let id = format!("A{}-{}-{}", p.a, p.towards, n);
        *n += 1;
        for node in &p.junction_nodes {
            let j = &model.junction_nodes[node];
            let ref_ = non_empty(j.tags.get("ref"));
            let name = non_empty(j.tags.get("name"));
            if ref_.is_none() && name.is_none() {
                continue;
            }
            if let Some(pr) = project_onto_polyline(&p.points, &p.cum_km, j.pos) {
                junctions.push(Junction {
                    node_id: *node,
                    pos: j.pos,
                    strand_id: id.clone(),
                    km: round_to(pr.km, 3).min(p.length_km),
                    ref_,
                    name,
                });
            }
        }
        strands.push(Strand {
            id,
            ref_: format!("A {}", p.a),
            towards: p.towards,
            polyline: p.polyline,
            points: p.points,
            cum_km: p.cum_km,
            length_km: p.length_km,
            first_way_id: p.first_way_id,
            way_ids: p.way_ids,
        });
    }
    strands.sort_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));
    junctions.sort_by(|a, b| {
        a.strand_id
            .as_bytes()
            .cmp(b.strand_id.as_bytes())
            .then(a.km.total_cmp(&b.km))
            .then_with(|| a.ref_.cmp(&b.ref_))
            .then_with(|| a.name.cmp(&b.name))
    });
    remnants.sort_by(|a, b| {
        a.ref_
            .cmp(&b.ref_)
            .then(b.length_km.total_cmp(&a.length_km))
            .then(a.way_ids.cmp(&b.way_ids))
    });
    StrandSet {
        strands,
        junctions,
        remnants,
    }
}

fn non_empty(v: Option<&str>) -> Option<String> {
    v.map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

#[cfg(test)]
mod tests {
    #[test]
    fn junction_text_normalization() {
        assert_eq!(normalize_list("52 b;52 a"), "52 a;52 b");
        assert_eq!(normalize_list(" B ; A;;"), "A;B");
        assert_eq!(display_list("Peine-Ost; Peine"), "Peine-Ost / Peine");
        assert_eq!(display_list("23"), "23");
        assert_eq!(
            display_exit(
                Some("24;23"),
                Some("Dreieck Delmenhorst;Delmenhorst-Hasport")
            ),
            "24 Dreieck Delmenhorst / 23 Delmenhorst-Hasport"
        );
        // unequal lengths: each joined
        assert_eq!(
            display_exit(Some("52"), Some("Peine-Ost;Peine")),
            "52 Peine-Ost / Peine"
        );
        assert_eq!(display_exit(Some("1;2"), Some("X")), "1 / 2 X");
        assert_eq!(
            display_exit(Some("38"), Some("Hamburg-Harburg")),
            "38 Hamburg-Harburg"
        );
        assert_eq!(display_exit(Some("38"), None), "38");
        assert_eq!(display_exit(None, Some("A;B")), "A / B");
        assert_eq!(display_exit(None, None), "");
    }

    use super::*;
    use crate::osm::model::{MotorwayWay, TaggedNode, Tags};

    /// Adds a straight way of `n` nodes north-south/east-west from `start`.
    #[allow(clippy::too_many_arguments)]
    fn add_way(
        model: &mut Model,
        id: i64,
        first_node: i64,
        start: LatLon,
        dlat: f64,
        dlon: f64,
        n: usize,
        oneway: Oneway,
    ) {
        let mut nodes = Vec::new();
        for i in 0..n {
            let nid = first_node + i as i64;
            model.coords.insert(
                nid,
                LatLon::new(start.lat + dlat * i as f64, start.lon + dlon * i as f64),
            );
            nodes.push(nid);
        }
        model.motorway_ways.insert(
            id,
            MotorwayWay {
                a_numbers: vec![1],
                oneway,
                nodes,
            },
        );
    }

    #[test]
    fn chains_two_ways_and_assigns_ids() {
        let mut m = Model::default();
        // Two consecutive northbound ways sharing node 12, ~3.3 km each.
        add_way(
            &mut m,
            10,
            1,
            LatLon::new(53.0, 10.0),
            0.003,
            0.0,
            11,
            Oneway::Forward,
        );
        add_way(
            &mut m,
            20,
            11,
            LatLon::new(53.03, 10.0),
            0.003,
            0.0,
            11,
            Oneway::Forward,
        );
        // Southbound carriageway.
        add_way(
            &mut m,
            30,
            100,
            LatLon::new(53.06, 10.0003),
            -0.003,
            0.0,
            21,
            Oneway::Forward,
        );
        let set = build(&m);
        assert_eq!(set.strands.len(), 2, "{:?}", set.remnants);
        assert_eq!(set.strands[0].id, "A1-N-0");
        assert_eq!(set.strands[0].way_ids, vec![10, 20]);
        assert_eq!(set.strands[1].id, "A1-S-0");
        assert!((set.strands[0].length_km - 6.672).abs() < 0.01);
        assert_eq!(set.strands[0].points.len(), 2); // straight line simplified
    }

    #[test]
    fn short_chains_are_remnants_and_reverse_works() {
        let mut m = Model::default();
        add_way(
            &mut m,
            10,
            1,
            LatLon::new(53.0, 10.0),
            0.001,
            0.0,
            6,
            Oneway::Forward,
        );
        // oneway=-1: node order is south->north in the file but travel is north->south.
        add_way(
            &mut m,
            20,
            100,
            LatLon::new(53.0, 10.0),
            0.003,
            0.0,
            11,
            Oneway::Reverse,
        );
        let set = build(&m);
        assert_eq!(set.remnants.len(), 1);
        assert_eq!(set.remnants[0].way_ids, vec![10]);
        assert_eq!(set.strands.len(), 1);
        assert_eq!(set.strands[0].towards, "S");
    }

    #[test]
    fn oneway_no_creates_both_directions_without_uturn_merge() {
        let mut m = Model::default();
        add_way(
            &mut m,
            10,
            1,
            LatLon::new(53.0, 10.0),
            0.003,
            0.0,
            11,
            Oneway::Both,
        );
        let set = build(&m);
        let ids: Vec<&str> = set.strands.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["A1-N-0", "A1-S-0"]);
    }

    #[test]
    fn longer_continuation_wins_and_junction_is_projected() {
        let mut m = Model::default();
        add_way(
            &mut m,
            10,
            1,
            LatLon::new(53.0, 10.0),
            0.003,
            0.0,
            11,
            Oneway::Forward,
        );
        // Node 11 is the end of way 10. Branch A: short (3.3 km), branch B: long (6.7 km).
        add_way(
            &mut m,
            20,
            11,
            LatLon::new(53.03, 10.0),
            0.003,
            0.0001,
            11,
            Oneway::Forward,
        );
        add_way(
            &mut m,
            30,
            11,
            LatLon::new(53.03, 10.0),
            0.003,
            -0.0001,
            21,
            Oneway::Forward,
        );
        m.coords.insert(11, LatLon::new(53.03, 10.0));
        // Junction on way 10 (node 6), without ref/name junction node 3.
        m.junction_nodes.insert(
            6,
            TaggedNode {
                pos: LatLon::new(53.015, 10.0),
                tags: Tags(vec![
                    ("ref".into(), "7".into()),
                    ("name".into(), " Test ".into()),
                ]),
            },
        );
        m.junction_nodes.insert(
            3,
            TaggedNode {
                pos: LatLon::new(53.006, 10.0),
                tags: Tags(vec![]),
            },
        );
        let set = build(&m);
        // Chain 1: 10 + 30 (long continuation); chain 2: 20 alone (3.3 km > 2 km).
        assert_eq!(set.strands.len(), 2);
        assert_eq!(set.strands[0].way_ids, vec![10, 30]);
        assert_eq!(set.strands[1].way_ids, vec![20]);
        assert_eq!(set.junctions.len(), 1);
        let j = &set.junctions[0];
        assert_eq!(j.strand_id, "A1-N-0");
        assert_eq!(j.ref_.as_deref(), Some("7"));
        assert_eq!(j.name.as_deref(), Some("Test"));
        assert!((j.km - 1.668).abs() < 0.01, "{}", j.km);
    }

    #[test]
    fn very_long_chain_does_not_overflow_the_stack() {
        // 40 000 consecutive 100 m ways (4 000 km): a recursive `down` would overflow the
        // 2 MB stack of the test thread.
        let mut m = Model::default();
        let n_ways = 40_000usize;
        for i in 0..n_ways {
            let lat0 = 40.0 + i as f64 * 0.0009;
            let first = i as i64 * 2 + 1;
            // Share the end node of way i with the start node of way i + 1.
            m.coords.insert(first, LatLon::new(lat0, 10.0));
            m.coords.insert(first + 2, LatLon::new(lat0 + 0.0009, 10.0));
            m.motorway_ways.insert(
                i as i64 + 1,
                MotorwayWay {
                    a_numbers: vec![1],
                    oneway: Oneway::Forward,
                    nodes: vec![first, first + 2],
                },
            );
        }
        let set = build(&m);
        assert_eq!(set.strands.len(), 1);
        assert_eq!(set.strands[0].way_ids.len(), n_ways);
    }

    #[test]
    fn closed_loop_terminates() {
        // A ring of four ways, one-way: the cycle guard keeps the DFS finite.
        let mut m = Model::default();
        let corners = [(53.0, 10.0), (53.03, 10.0), (53.03, 10.04), (53.0, 10.04)];
        for (i, c) in corners.iter().enumerate() {
            m.coords.insert(i as i64 + 1, LatLon::new(c.0, c.1));
        }
        for i in 0..4usize {
            m.motorway_ways.insert(
                i as i64 + 10,
                MotorwayWay {
                    a_numbers: vec![1],
                    oneway: Oneway::Forward,
                    nodes: vec![i as i64 + 1, (i as i64 + 1) % 4 + 1],
                },
            );
        }
        let set = build(&m);
        let total: usize = set.strands.iter().map(|s| s.way_ids.len()).sum();
        assert_eq!(
            total + set.remnants.iter().map(|r| r.way_ids.len()).sum::<usize>(),
            4
        );
    }
}
