//! Named constants for every numeric threshold of `docs/heuristics.md`.
//! Values must not be changed here without a change in the heuristics document.
#![allow(dead_code)] // Several constants are only used from phase 2 on.

/// Mean earth radius in metres (docs/schema.md §3).
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;

// --- Strands (heuristics §3) ---
/// Chains up to this length (km) are reported as remnants, longer ones become strands.
pub const MIN_STRAND_LENGTH_KM: f64 = 2.0;
/// A successor that turns the direction by more than this angle is not a continuation.
pub const UTURN_GUARD_DEG: f64 = 150.0;
/// Douglas-Peucker tolerance in metres.
pub const SIMPLIFY_TOLERANCE_M: f64 = 15.0;
/// Opposite direction: local bearing difference above this angle.
pub const OPPOSITE_DIRECTION_DEG: f64 = 135.0;

// --- Polyline / rounding (schema §2, §3) ---
pub const POLYLINE_FACTOR: f64 = 1e5;

// --- Rastanlage assignment (heuristics §4) ---
pub const CANDIDATE_MAX_DISTANCE_M: f64 = 300.0;
pub const BOTH_SIDES_DISTANCE_M: f64 = 100.0;
pub const SHARED_SECTION_TOLERANCE_M: f64 = 5.0;
/// Services objects farther than this from the chosen strand are deferred (§4.1a, P-01).
pub const RASTANLAGE_MAX_DISTANCE_M: f64 = 150.0;
/// Name/operator/brand substrings (lowercase) that mark an Autohof-like object (§4.1a, P-01).
/// Rule 2 keywords (lowercase, D-33): `Autohof`, `Truckstop`, `Truck Stop`.
pub const AUTOHOF_RULE2_KEYWORDS: [&str; 3] = ["autohof", "truckstop", "truck stop"];
/// Autohof sites on the same strand at most this far apart are one site (D-33); an Autohof at
/// most this far from a Rastanlage centroid is dropped.
pub const AUTOHOF_SITE_DEDUPE_M: f64 = 150.0;
pub const AUTOHOF_LIKE_KEYWORDS: [&str; 3] = ["autohof", "truck", "maxi"];
/// Duplicate merging: maximum km difference on the same strand (§4.1a, P-02).
pub const MERGE_MAX_KM_DIFF: f64 = 0.35;
/// Both sides: a polygon vertex farther left than this reaches beyond its own carriageway (§4.3).
pub const BOTH_SIDES_VERTEX_LEFT_M: f64 = 15.0;

// --- POI corridor (heuristics §6) ---
pub const CORRIDOR_VERTEX_WINDOW_KM: f64 = 2.0;
pub const CORRIDOR_OUTER_BUFFER_M: f64 = 50.0;
pub const NODE_CORRIDOR_HALF_LENGTH_KM: f64 = 0.150;
pub const NODE_CORRIDOR_INNER_M: f64 = 30.0;
pub const NODE_CORRIDOR_OUTER_M: f64 = 50.0;
pub const FAST_CHARGER_MIN_KW: f64 = 50.0;
/// Brand IDs that imply a fast charger (heuristics §6.4, P-04).
pub const FAST_CHARGER_BRANDS: [&str; 3] = ["ionity", "tesla-supercharger", "fastned"];
/// Whitespace-free, lowercase spellings that count as open 24 h (heuristics §6.4, P-05).
pub const OPEN_24H_SPELLINGS: [&str; 4] =
    ["24/7", "mo-su00:00-24:00", "mo-su0:00-24:00", "00:00-24:00"];

// --- Autohoefe (heuristics §7) ---
pub const JUNCTION_CLUSTER_DISTANCE_KM: f64 = 1.5;
pub const AUTOHOF_MAX_DISTANCE_KM: f64 = 2.0;
pub const AUTOHOF_POI_RADIUS_M: f64 = 150.0;
pub const AUTOHOF_DETOUR_FACTOR: f64 = 1.3;
/// Rule 3 (D-32): a truck parking must be at most this far from the fuel station.
pub const AUTOHOF_TRUCK_PARKING_RADIUS_M: f64 = 300.0;
/// Rule 3 (D-32): maximum straight-line distance to the nearest junction node of the cluster.
pub const AUTOHOF_RULE3_MAX_JUNCTION_KM: f64 = 1.0;
/// `hgv=*` values that make an `amenity=parking` a truck parking (D-32); `capacity:hgv` > 0
/// does as well.
pub const TRUCK_PARKING_HGV_VALUES: [&str; 4] = ["yes", "designated", "only", "official"];
/// Rest-area exits: junction names starting with one of these (lowercase) are not
/// Anschlussstellen (D-32); only deferred services objects may use such clusters. A bare `Rast`
/// prefix would also hit the town Rastede (A 29), hence the listed words.
pub const REST_AREA_JUNCTION_PREFIXES: [&str; 5] = [
    "rastplatz",
    "raststätte",
    "raststaette",
    "rasthof",
    "rastanlage",
];

// --- Report (heuristics §9) ---
pub const SAMPLE_SEED: u64 = 20_261_019;
pub const SAMPLE_COUNT: usize = 20;
