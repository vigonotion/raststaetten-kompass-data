//! Own geometry code (no geo crates). Conventions: docs/schema.md §3.

pub mod distance;
pub mod local;
pub mod polygon;
pub mod polyline;
pub mod simplify;

/// WGS84 position in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    pub const fn new(lat: f64, lon: f64) -> Self {
        LatLon { lat, lon }
    }
}

/// Rounds to `decimals` decimal places (output rounding only).
pub fn round_to(value: f64, decimals: i32) -> f64 {
    let f = 10f64.powi(decimals);
    (value * f).round() / f
}
