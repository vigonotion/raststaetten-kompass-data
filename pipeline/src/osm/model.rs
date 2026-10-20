//! In-memory model of the OSM objects collected in pass 1 / pass 2.
//! Fields marked "phase 2" are collected now but only consumed later.
#![allow(dead_code)]

use crate::geo::LatLon;
use std::collections::{BTreeMap, HashMap};

/// A small set of selected tags (only keys the pipeline cares about).
#[derive(Clone, Debug, Default)]
pub struct Tags(pub Vec<(String, String)>);

impl Tags {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oneway {
    /// Node order is the direction of travel (default for motorways).
    Forward,
    /// `oneway=-1`: reverse the node order.
    Reverse,
    /// `oneway=no`: both directions.
    Both,
}

#[derive(Clone, Debug)]
pub struct MotorwayWay {
    /// Normalized A numbers of the `ref` tag, sorted, unique.
    pub a_numbers: Vec<u32>,
    pub oneway: Oneway,
    pub nodes: Vec<i64>,
}

#[derive(Clone, Debug)]
pub struct TaggedNode {
    pub pos: LatLon,
    pub tags: Tags,
}

#[derive(Clone, Debug)]
pub struct TaggedWay {
    pub nodes: Vec<i64>,
    pub tags: Tags,
}

#[derive(Clone, Debug)]
pub struct ServiceRelation {
    pub tags: Tags,
    /// Way IDs of members with role `outer` or empty role.
    pub outer_ways: Vec<i64>,
}

/// Everything collected from the inputs. Maps are keyed by OSM ID, so inserting the same
/// object from several inputs deduplicates by (type, id) (PLAN D-04).
#[derive(Default)]
pub struct Model {
    pub motorway_ways: BTreeMap<i64, MotorwayWay>,
    /// Closed ways with `highway=services` (phase 2).
    pub service_ways: BTreeMap<i64, TaggedWay>,
    /// `type=multipolygon` relations with `highway=services` (phase 2).
    pub service_relations: BTreeMap<i64, ServiceRelation>,
    /// Node lists of ways that are outer members of service relations (phase 2).
    pub member_ways: BTreeMap<i64, Vec<i64>>,
    /// Nodes with `highway=services` (phase 2).
    pub service_nodes: BTreeMap<i64, TaggedNode>,
    pub junction_nodes: BTreeMap<i64, TaggedNode>,
    /// POI nodes (phase 2).
    pub poi_nodes: BTreeMap<i64, TaggedNode>,
    /// Closed POI ways, located via their centroid later (phase 2).
    pub poi_ways: BTreeMap<i64, TaggedWay>,
    /// Truck parkings as nodes (`amenity=parking` with an hgv tag, D-32).
    pub truck_parking_nodes: BTreeMap<i64, LatLon>,
    /// Truck parkings as closed ways (node lists; located via their centroid later).
    pub truck_parking_ways: BTreeMap<i64, Vec<i64>>,
    /// Coordinates of the node IDs needed by ways (pass 2).
    pub coords: HashMap<i64, LatLon>,
    /// Objects that were skipped because an earlier input already contained them.
    pub duplicates: usize,
}

/// Statistics of one pass for the report.
#[derive(Clone, Debug, Default)]
pub struct PassStats {
    pub name: String,
    pub seconds: f64,
    /// Peak RSS (VmHWM) in kB after the pass, if available.
    pub peak_rss_kb: Option<u64>,
}
