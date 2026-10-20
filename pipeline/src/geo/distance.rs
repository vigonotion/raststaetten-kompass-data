//! Haversine distance, bearings and sector mapping.

use super::LatLon;
use crate::params::EARTH_RADIUS_M;

/// Great-circle distance in metres.
pub fn haversine_m(a: LatLon, b: LatLon) -> f64 {
    let phi1 = a.lat.to_radians();
    let phi2 = b.lat.to_radians();
    let dphi = phi2 - phi1;
    let dlambda = (b.lon - a.lon).to_radians();
    let h = (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// Initial great-circle bearing in degrees `[0, 360)`, 0 = north, clockwise.
pub fn bearing_deg(a: LatLon, b: LatLon) -> f64 {
    let phi1 = a.lat.to_radians();
    let phi2 = b.lat.to_radians();
    let dlambda = (b.lon - a.lon).to_radians();
    let y = dlambda.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * dlambda.cos();
    let deg = y.atan2(x).to_degrees();
    let r = deg.rem_euclid(360.0);
    if r >= 360.0 {
        0.0
    } else {
        r
    }
}

const SECTORS: [&str; 8] = ["N", "NO", "O", "SO", "S", "SW", "W", "NW"];

/// Maps a bearing to one of 8 sectors of 45 degrees centred on the direction.
pub fn towards(bearing: f64) -> &'static str {
    let b = bearing.rem_euclid(360.0);
    let idx = (((b + 22.5) / 45.0).floor() as usize) % 8;
    SECTORS[idx]
}

/// Smallest absolute difference of two bearings, in `[0, 180]`.
pub fn angle_diff_deg(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 {
        360.0 - d
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_one_degree_latitude() {
        let d = haversine_m(LatLon::new(50.0, 10.0), LatLon::new(51.0, 10.0));
        let expected = EARTH_RADIUS_M * 1f64.to_radians();
        assert!((d - expected).abs() < 1e-6, "{d} vs {expected}");
        assert!((d - 111_195.08).abs() < 0.1);
    }

    #[test]
    fn haversine_zero_and_symmetry() {
        let a = LatLon::new(53.55, 9.99);
        let b = LatLon::new(52.52, 13.40);
        assert_eq!(haversine_m(a, a), 0.0);
        assert!((haversine_m(a, b) - haversine_m(b, a)).abs() < 1e-9);
        // Hamburg - Berlin is roughly 255 km.
        assert!((haversine_m(a, b) / 1000.0 - 255.0).abs() < 5.0);
    }

    #[test]
    fn bearing_cardinals() {
        let o = LatLon::new(50.0, 10.0);
        assert!((bearing_deg(o, LatLon::new(51.0, 10.0)) - 0.0).abs() < 1e-9);
        assert!((bearing_deg(o, LatLon::new(49.0, 10.0)) - 180.0).abs() < 1e-9);
        let e = bearing_deg(o, LatLon::new(50.0, 11.0));
        assert!((e - 90.0).abs() < 0.5, "{e}");
        let w = bearing_deg(o, LatLon::new(50.0, 9.0));
        assert!((w - 270.0).abs() < 0.5, "{w}");
    }

    #[test]
    fn towards_sectors_and_boundaries() {
        assert_eq!(towards(0.0), "N");
        assert_eq!(towards(22.4999), "N");
        assert_eq!(towards(22.5), "NO");
        assert_eq!(towards(45.0), "NO");
        assert_eq!(towards(67.5), "O");
        assert_eq!(towards(90.0), "O");
        assert_eq!(towards(135.0), "SO");
        assert_eq!(towards(180.0), "S");
        assert_eq!(towards(225.0), "SW");
        assert_eq!(towards(270.0), "W");
        assert_eq!(towards(315.0), "NW");
        assert_eq!(towards(337.4999), "NW");
        assert_eq!(towards(337.5), "N");
        assert_eq!(towards(359.9), "N");
    }

    #[test]
    fn angle_diff() {
        assert_eq!(angle_diff_deg(10.0, 350.0), 20.0);
        assert_eq!(angle_diff_deg(0.0, 180.0), 180.0);
        assert_eq!(angle_diff_deg(90.0, 90.0), 0.0);
    }
}
