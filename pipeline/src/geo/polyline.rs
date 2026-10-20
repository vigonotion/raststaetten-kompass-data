//! Google Encoded Polyline Algorithm Format, precision 1e5, order lat, lon.

use super::LatLon;
use crate::params::POLYLINE_FACTOR;

fn encode_value(out: &mut String, value: i64) {
    let mut v = if value < 0 { !(value << 1) } else { value << 1 } as u64;
    while v >= 0x20 {
        out.push((((0x20 | (v & 0x1f)) + 63) as u8) as char);
        v >>= 5;
    }
    out.push(((v + 63) as u8) as char);
}

pub fn encode(points: &[LatLon]) -> String {
    let mut out = String::new();
    let (mut prev_lat, mut prev_lon) = (0i64, 0i64);
    for p in points {
        let lat = (p.lat * POLYLINE_FACTOR).round() as i64;
        let lon = (p.lon * POLYLINE_FACTOR).round() as i64;
        encode_value(&mut out, lat - prev_lat);
        encode_value(&mut out, lon - prev_lon);
        prev_lat = lat;
        prev_lon = lon;
    }
    out
}

/// Decodes a polyline; `Err` describes malformed input.
pub fn decode(s: &str) -> Result<Vec<LatLon>, String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let next = |i: &mut usize| -> Result<i64, String> {
        let mut result: u64 = 0;
        let mut shift = 0;
        loop {
            let b = *bytes.get(*i).ok_or("truncated polyline")?;
            if !(63..=126).contains(&b) {
                return Err(format!("invalid polyline character {:?}", b as char));
            }
            *i += 1;
            let chunk = (b - 63) as u64;
            if shift > 60 {
                return Err("polyline value too large".to_string());
            }
            result |= (chunk & 0x1f) << shift;
            shift += 5;
            if chunk & 0x20 == 0 {
                break;
            }
        }
        Ok(if result & 1 == 1 {
            !((result >> 1) as i64)
        } else {
            (result >> 1) as i64
        })
    };
    let mut pts = Vec::new();
    let (mut lat, mut lon) = (0i64, 0i64);
    while i < bytes.len() {
        lat += next(&mut i)?;
        lon += next(&mut i)?;
        pts.push(LatLon::new(
            lat as f64 / POLYLINE_FACTOR,
            lon as f64 / POLYLINE_FACTOR,
        ));
    }
    Ok(pts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::local::cumulative_km;
    use crate::geo::round_to;

    #[test]
    fn google_reference_example() {
        let pts = [
            LatLon::new(38.5, -120.2),
            LatLon::new(40.7, -120.95),
            LatLon::new(43.252, -126.453),
        ];
        assert_eq!(encode(&pts), "_p~iF~ps|U_ulLnnqC_mqNvxq`@");
        let dec = decode("_p~iF~ps|U_ulLnnqC_mqNvxq`@").unwrap();
        assert_eq!(dec, pts.to_vec());
    }

    #[test]
    fn round_trip() {
        let pts: Vec<LatLon> = (0..200)
            .map(|i| {
                let t = i as f64;
                LatLon::new(
                    53.5 + (t * 0.37).sin() * 0.3 - t * 0.0011,
                    9.9 + (t * 0.21).cos() * 0.4 + t * 0.0007,
                )
            })
            .collect();
        let dec = decode(&encode(&pts)).unwrap();
        assert_eq!(dec.len(), pts.len());
        for (a, b) in pts.iter().zip(&dec) {
            assert!((a.lat - b.lat).abs() <= 0.5e-5 + 1e-12);
            assert!((a.lon - b.lon).abs() <= 0.5e-5 + 1e-12);
        }
        // Encoding decoded points is a fixed point.
        assert_eq!(encode(&dec), encode(&pts));
    }

    #[test]
    fn negative_and_empty() {
        let pts = [
            LatLon::new(-33.86785, 151.20732),
            LatLon::new(-0.00001, -179.99999),
        ];
        assert_eq!(decode(&encode(&pts)).unwrap(), pts.to_vec());
        assert_eq!(encode(&[]), "");
        assert!(decode("").unwrap().is_empty());
    }

    #[test]
    fn malformed() {
        assert!(decode("_p~iF").is_err());
        assert!(decode("\u{20}abc").is_err());
    }

    #[test]
    fn example_dataset_lengths_match() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/example-dataset.json");
        let text = std::fs::read_to_string(path).expect("example dataset");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let strands = v["strands"].as_array().unwrap();
        assert!(!strands.is_empty());
        for s in strands {
            let pts = decode(s["polyline"].as_str().unwrap()).unwrap();
            let len = round_to(*cumulative_km(&pts).last().unwrap(), 3);
            assert_eq!(len, s["lengthKm"].as_f64().unwrap(), "strand {}", s["id"]);
        }
    }
}
