//! Two-pass PBF reading with osmpbf.

use super::model::*;
use crate::error::{Error, Result};
use crate::geo::LatLon;
use crate::params::TRUCK_PARKING_HGV_VALUES;
use osmpbf::{BlobDecode, BlobReader, Element, ElementReader, RelMemberType};
use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

const POI_AMENITIES: [&str; 6] = [
    "fast_food",
    "restaurant",
    "cafe",
    "fuel",
    "charging_station",
    "shower",
];

/// Parses the `ref` tag into normalized A numbers (heuristics §2).
pub fn parse_a_numbers(ref_tag: &str) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for part in ref_tag.split(';') {
        let part = part.trim();
        let mut chars = part.chars();
        match chars.next() {
            Some('A') | Some('a') => {}
            _ => continue,
        }
        let rest = chars.as_str().trim_start();
        if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(n) = rest.parse::<u32>() {
            if n > 0 && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out.sort_unstable();
    out
}

fn is_relevant_key(k: &str) -> bool {
    matches!(
        k,
        "highway"
            | "amenity"
            | "brand"
            | "name"
            | "operator"
            | "ref"
            | "opening_hours"
            | "shower"
            | "maxpower"
            | "type"
    ) || k.starts_with("socket:")
}

fn collect_tags<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Tags {
    Tags(
        tags.filter(|(k, _)| is_relevant_key(k))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

/// Quick classification of an element's tags without allocating.
#[derive(Default)]
struct Quick<'a> {
    highway: Option<&'a str>,
    amenity: Option<&'a str>,
    has_brand: bool,
    ref_tag: Option<&'a str>,
    oneway: Option<&'a str>,
    hgv: Option<&'a str>,
    capacity_hgv: Option<&'a str>,
    rel_type: Option<&'a str>,
}

fn quick<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Quick<'a> {
    let mut q = Quick::default();
    for (k, v) in tags {
        match k {
            "highway" => q.highway = Some(v),
            "amenity" => q.amenity = Some(v),
            "brand" => q.has_brand = true,
            "ref" => q.ref_tag = Some(v),
            "oneway" => q.oneway = Some(v),
            "type" => q.rel_type = Some(v),
            "hgv" => q.hgv = Some(v),
            "capacity:hgv" => q.capacity_hgv = Some(v),
            _ => {}
        }
    }
    q
}

/// Truck parking (heuristics §7, D-32): `amenity=parking` with `hgv` in
/// `TRUCK_PARKING_HGV_VALUES` or `capacity:hgv` > 0.
pub fn is_truck_parking(
    amenity: Option<&str>,
    hgv: Option<&str>,
    capacity_hgv: Option<&str>,
) -> bool {
    amenity == Some("parking")
        && (hgv.is_some_and(|v| TRUCK_PARKING_HGV_VALUES.contains(&v.trim()))
            || capacity_hgv.is_some_and(|v| v.trim().parse::<u32>().is_ok_and(|n| n > 0)))
}

impl Quick<'_> {
    fn is_truck_parking(&self) -> bool {
        is_truck_parking(self.amenity, self.hgv, self.capacity_hgv)
    }

    fn is_poi(&self) -> bool {
        self.has_brand || self.amenity.is_some_and(|a| POI_AMENITIES.contains(&a))
    }
}

/// Reads the replication timestamp (seconds since epoch) of the header block, if present.
pub fn read_header_timestamp(path: &str) -> Result<Option<i64>> {
    let reader =
        BlobReader::from_path(path).map_err(|e| Error::Osm(format!("cannot open {path}"), e))?;
    for blob in reader {
        let blob = blob.map_err(|e| Error::Osm(format!("cannot read {path}"), e))?;
        if let BlobDecode::OsmHeader(h) = blob
            .decode()
            .map_err(|e| Error::Osm(format!("cannot decode header of {path}"), e))?
        {
            return Ok(h.osmosis_replication_timestamp());
        }
    }
    Ok(None)
}

/// Peak resident set size in kB from `/proc/self/status` (Linux only).
pub fn peak_rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find(|l| l.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn insert_new<V>(map: &mut BTreeMap<i64, V>, id: i64, v: V, dups: &mut usize) {
    use std::collections::btree_map::Entry;
    match map.entry(id) {
        Entry::Vacant(e) => {
            e.insert(v);
        }
        Entry::Occupied(_) => *dups += 1,
    }
}

fn scan<F>(path: &str, f: F) -> Result<()>
where
    F: for<'a> FnMut(Element<'a>),
{
    ElementReader::from_path(path)
        .map_err(|e| Error::Osm(format!("cannot open {path}"), e))?
        .for_each(f)
        .map_err(|e| Error::Osm(format!("cannot read {path}"), e))
}

/// Pass 1: motorway ways, services, junction nodes, POIs.
pub fn pass1(model: &mut Model, path: &str) -> Result<()> {
    let mut dups = 0usize;
    scan(path, |el| match el {
        Element::Node(n) => {
            let q = quick(n.tags());
            node_dispatch(
                model,
                &mut dups,
                n.id(),
                LatLon::new(n.lat(), n.lon()),
                &q,
                || collect_tags(n.tags()),
            );
        }
        Element::DenseNode(n) => {
            let q = quick(n.tags());
            node_dispatch(
                model,
                &mut dups,
                n.id(),
                LatLon::new(n.lat(), n.lon()),
                &q,
                || collect_tags(n.tags()),
            );
        }
        Element::Way(w) => {
            let q = quick(w.tags());
            if q.highway == Some("motorway") {
                let a_numbers = q.ref_tag.map(parse_a_numbers).unwrap_or_default();
                if !a_numbers.is_empty() {
                    let oneway = match q.oneway {
                        Some("-1") => Oneway::Reverse,
                        Some("no") => Oneway::Both,
                        _ => Oneway::Forward,
                    };
                    let mw = MotorwayWay {
                        a_numbers,
                        oneway,
                        nodes: w.refs().collect(),
                    };
                    insert_new(&mut model.motorway_ways, w.id(), mw, &mut dups);
                }
                return;
            }
            let nodes: Vec<i64> = w.refs().collect();
            let closed = nodes.len() >= 4 && nodes.first() == nodes.last();
            if q.highway == Some("services") && closed {
                let tw = TaggedWay {
                    nodes: nodes.clone(),
                    tags: collect_tags(w.tags()),
                };
                insert_new(&mut model.service_ways, w.id(), tw, &mut dups);
            }
            if q.is_truck_parking() && closed {
                insert_new(
                    &mut model.truck_parking_ways,
                    w.id(),
                    nodes.clone(),
                    &mut dups,
                );
            }
            if q.is_poi() && closed {
                let tw = TaggedWay {
                    nodes,
                    tags: collect_tags(w.tags()),
                };
                insert_new(&mut model.poi_ways, w.id(), tw, &mut dups);
            }
        }
        Element::Relation(r) => {
            let q = quick(r.tags());
            if q.highway == Some("services") && q.rel_type == Some("multipolygon") {
                let outer_ways: Vec<i64> = r
                    .members()
                    .filter(|m| m.member_type == RelMemberType::Way)
                    .filter(|m| matches!(m.role(), Ok("outer") | Ok("")))
                    .map(|m| m.member_id)
                    .collect();
                let sr = ServiceRelation {
                    tags: collect_tags(r.tags()),
                    outer_ways,
                };
                insert_new(&mut model.service_relations, r.id(), sr, &mut dups);
            }
        }
    })?;
    model.duplicates += dups;
    Ok(())
}

fn node_dispatch(
    model: &mut Model,
    dups: &mut usize,
    id: i64,
    pos: LatLon,
    q: &Quick<'_>,
    tags: impl Fn() -> Tags,
) {
    if q.highway.is_none() && q.amenity.is_none() && !q.has_brand {
        return;
    }
    if q.highway == Some("motorway_junction") {
        let tn = TaggedNode { pos, tags: tags() };
        insert_new(&mut model.junction_nodes, id, tn, dups);
    }
    if q.highway == Some("services") {
        let tn = TaggedNode { pos, tags: tags() };
        insert_new(&mut model.service_nodes, id, tn, dups);
    }
    if q.is_poi() {
        let tn = TaggedNode { pos, tags: tags() };
        insert_new(&mut model.poi_nodes, id, tn, dups);
    }
    if q.is_truck_parking() {
        insert_new(&mut model.truck_parking_nodes, id, pos, dups);
    }
}

/// Pass 1b: node lists of ways that are outer members of service relations
/// (relations may reference ways on either side of them in the file; the alternative of storing
/// all closed ways in pass 1 would cost gigabytes for Germany).
pub fn pass1b(model: &mut Model, path: &str) -> Result<()> {
    let wanted: HashSet<i64> = model
        .service_relations
        .values()
        .flat_map(|r| r.outer_ways.iter().copied())
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let mut found: Vec<(i64, Vec<i64>)> = Vec::new();
    scan(path, |el| {
        if let Element::Way(w) = el {
            if wanted.contains(&w.id()) {
                found.push((w.id(), w.refs().collect()));
            }
        }
    })?;
    let mut dups = 0;
    for (id, nodes) in found {
        insert_new(&mut model.member_ways, id, nodes, &mut dups);
    }
    // Duplicates across inputs are expected and not counted separately here.
    Ok(())
}

/// Pass 2: loads the coordinates of exactly the node IDs referenced by collected ways.
pub fn pass2(model: &mut Model, paths: &[String]) -> Result<()> {
    let mut needed: HashSet<i64> = HashSet::new();
    for w in model.motorway_ways.values() {
        needed.extend(w.nodes.iter().copied());
    }
    for w in model.service_ways.values().chain(model.poi_ways.values()) {
        needed.extend(w.nodes.iter().copied());
    }
    for nodes in model
        .member_ways
        .values()
        .chain(model.truck_parking_ways.values())
    {
        needed.extend(nodes.iter().copied());
    }
    let mut coords = std::mem::take(&mut model.coords);
    coords.reserve(needed.len());
    for path in paths {
        scan(path, |el| match el {
            Element::Node(n) if needed.contains(&n.id()) => {
                coords.insert(n.id(), LatLon::new(n.lat(), n.lon()));
            }
            Element::DenseNode(n) if needed.contains(&n.id()) => {
                coords.insert(n.id(), LatLon::new(n.lat(), n.lon()));
            }
            _ => {}
        })?;
    }
    model.coords = coords;
    Ok(())
}

/// Runs all passes over all inputs and returns the model and per-pass statistics.
pub fn read_all(paths: &[String]) -> Result<(Model, Vec<PassStats>)> {
    let mut model = Model::default();
    let mut stats = Vec::new();

    let t = Instant::now();
    for p in paths {
        pass1(&mut model, p)?;
    }
    stats.push(PassStats {
        name: "Pass 1 (collect ways, services, junctions, POIs)".into(),
        seconds: t.elapsed().as_secs_f64(),
        peak_rss_kb: peak_rss_kb(),
    });

    let t = Instant::now();
    for p in paths {
        pass1b(&mut model, p)?;
    }
    stats.push(PassStats {
        name: "Pass 1b (node lists of service relation members)".into(),
        seconds: t.elapsed().as_secs_f64(),
        peak_rss_kb: peak_rss_kb(),
    });

    let t = Instant::now();
    pass2(&mut model, paths)?;
    stats.push(PassStats {
        name: "Pass 2 (node coordinates)".into(),
        seconds: t.elapsed().as_secs_f64(),
        peak_rss_kb: peak_rss_kb(),
    });
    Ok((model, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truck_parking_tag_set() {
        for v in ["yes", "designated", "only", "official"] {
            assert!(is_truck_parking(Some("parking"), Some(v), None), "{v}");
        }
        for v in ["no", "customers", "delivery", "private", "destination"] {
            assert!(!is_truck_parking(Some("parking"), Some(v), None), "{v}");
        }
        assert!(is_truck_parking(Some("parking"), None, Some("12")));
        assert!(is_truck_parking(Some("parking"), Some("no"), Some("3")));
        assert!(!is_truck_parking(Some("parking"), None, Some("0")));
        assert!(!is_truck_parking(Some("parking"), None, Some("yes")));
        assert!(!is_truck_parking(Some("parking"), None, None));
        assert!(!is_truck_parking(Some("fuel"), Some("yes"), Some("5")));
        assert!(!is_truck_parking(None, Some("yes"), None));
    }

    #[test]
    fn a_numbers() {
        assert_eq!(parse_a_numbers("A 1"), vec![1]);
        assert_eq!(parse_a_numbers("A1"), vec![1]);
        assert_eq!(parse_a_numbers("a  7"), vec![7]);
        assert_eq!(parse_a_numbers("A 01"), vec![1]);
        assert_eq!(parse_a_numbers("A 1;A 3"), vec![1, 3]);
        assert_eq!(parse_a_numbers("A 3; A 1"), vec![1, 3]);
        assert_eq!(parse_a_numbers("A 1;E 37"), vec![1]);
        assert_eq!(parse_a_numbers("E 45"), Vec::<u32>::new());
        assert_eq!(parse_a_numbers("A 7a"), Vec::<u32>::new());
        assert_eq!(parse_a_numbers("A"), Vec::<u32>::new());
        assert_eq!(parse_a_numbers("A 0"), Vec::<u32>::new());
        assert_eq!(parse_a_numbers("B 3"), Vec::<u32>::new());
        assert_eq!(parse_a_numbers("A 1;A 1"), vec![1]);
    }
}
