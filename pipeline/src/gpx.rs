//! `rastpipe gpx`: deterministic test drives from real strand geometry (heuristics §11).

use crate::error::{Error, Result};
use crate::geo::distance::bearing_deg;
use crate::geo::local::{cumulative_km, LocalProj};
use crate::geo::{polyline, LatLon};
use crate::output::format_rfc3339;
use crate::prng::SplitMix64;
use serde::Serialize;
use serde_json::Value;
use std::fmt::Write;

/// 2026-01-01T00:00:00Z as Unix time.
const START_TS: i64 = 1_767_225_600;
const INTERVAL_S: u32 = 1;
const EXPECTATIONS: [&str; 3] = ["matched", "notOnMotorway", "tooSlow"];

/// A decoded strand from a dataset file.
pub struct StrandGeom {
    pub id: String,
    pub points: Vec<LatLon>,
    pub cum_km: Vec<f64>,
    pub length_km: f64,
}

pub fn load_strands(dataset_path: &str) -> Result<Vec<StrandGeom>> {
    let text = std::fs::read_to_string(dataset_path)
        .map_err(|e| Error::Io(format!("cannot read {dataset_path}"), e))?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Json(format!("cannot parse {dataset_path}"), e))?;
    let arr = v
        .get("strands")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Data(format!("{dataset_path}: `strands` missing")))?;
    let mut out = Vec::new();
    for s in arr {
        let id = s.get("id").and_then(Value::as_str);
        let poly = s.get("polyline").and_then(Value::as_str);
        let (Some(id), Some(poly)) = (id, poly) else {
            return Err(Error::Data(format!("{dataset_path}: malformed strand")));
        };
        let points = polyline::decode(poly)
            .map_err(|e| Error::Data(format!("{dataset_path}: strand {id}: {e}")))?;
        let cum_km = cumulative_km(&points);
        let length_km = cum_km.last().copied().unwrap_or(0.0);
        out.push(StrandGeom {
            id: id.to_string(),
            points,
            cum_km,
            length_km,
        });
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct LegSpec {
    pub strand: String,
    pub from_km: f64,
    pub to_km: f64,
}

#[derive(Clone, Debug)]
pub struct DriveSpec {
    pub legs: Vec<LegSpec>,
    pub speed_kmh: f64,
    pub offset_m: f64,
    pub noise_m: f64,
    pub seed: u64,
    pub expect: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarLeg {
    pub strand_id: String,
    pub from_km: f64,
    pub to_km: f64,
    pub first_point: usize,
    pub last_point: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidecar {
    pub name: String,
    pub seed: u64,
    pub speed_kmh: f64,
    pub offset_m: f64,
    pub noise_m: f64,
    pub interval_s: u32,
    pub legs: Vec<SidecarLeg>,
    pub expect: String,
}

pub struct Drive {
    pub points: Vec<LatLon>,
    pub legs: Vec<SidecarLeg>,
}

/// Position at `km` on the strand plus the travel direction (unit vector east/north) there.
fn position_at(s: &StrandGeom, km: f64) -> (LatLon, f64) {
    let n_seg = s.points.len() - 1;
    let i = s
        .cum_km
        .partition_point(|&c| c <= km)
        .saturating_sub(1)
        .min(n_seg - 1);
    let seg_len = s.cum_km[i + 1] - s.cum_km[i];
    let t = if seg_len > 0.0 {
        ((km - s.cum_km[i]) / seg_len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (a, b) = (s.points[i], s.points[i + 1]);
    (
        LatLon::new(a.lat + t * (b.lat - a.lat), a.lon + t * (b.lon - a.lon)),
        bearing_deg(a, b),
    )
}

pub fn validate_spec(spec: &DriveSpec, strands: &[StrandGeom]) -> Result<()> {
    if spec.legs.is_empty() {
        return Err(Error::Usage("at least one --strand leg is required".into()));
    }
    if spec.speed_kmh.is_nan() || spec.speed_kmh <= 0.0 {
        return Err(Error::Usage("--speed-kmh must be > 0".into()));
    }
    if spec.noise_m < 0.0 {
        return Err(Error::Usage("--noise-m must be >= 0".into()));
    }
    if !EXPECTATIONS.contains(&spec.expect.as_str()) {
        return Err(Error::Usage(format!(
            "--expect must be one of {EXPECTATIONS:?}"
        )));
    }
    for (i, l) in spec.legs.iter().enumerate() {
        let Some(s) = strands.iter().find(|s| s.id == l.strand) else {
            return Err(Error::Usage(format!(
                "leg {}: strand `{}` not found in the dataset",
                i + 1,
                l.strand
            )));
        };
        // lengthKm in the dataset is rounded to 3 decimals; allow that rounding.
        if !(l.from_km >= 0.0 && l.from_km < l.to_km && l.to_km <= s.length_km + 0.0005) {
            return Err(Error::Usage(format!(
                "leg {}: need 0 <= from-km < to-km <= {:.3} (strand {}), got {} .. {}",
                i + 1,
                s.length_km,
                s.id,
                l.from_km,
                l.to_km
            )));
        }
    }
    Ok(())
}

/// Generates the track. Same arguments and seed give identical points.
pub fn generate(spec: &DriveSpec, strands: &[StrandGeom]) -> Result<Drive> {
    validate_spec(spec, strands)?;
    let step_m = spec.speed_kmh / 3.6 * f64::from(INTERVAL_S);
    let mut rng = SplitMix64::new(spec.seed);
    let mut points: Vec<LatLon> = Vec::new();
    let mut legs = Vec::new();
    for l in &spec.legs {
        let s = strands
            .iter()
            .find(|s| s.id == l.strand)
            .expect("validated");
        let n = ((l.to_km - l.from_km) * 1000.0 / step_m + 1e-9).floor() as usize + 1;
        let first = points.len();
        for k in 0..n {
            let km = (l.from_km + k as f64 * step_m / 1000.0).min(s.length_km);
            let (base, bearing) = position_at(s, km);
            let proj = LocalProj::new(base);
            // Right of the direction of travel: (sin(b + 90), cos(b + 90)) = (cos b, -sin b).
            let b = bearing.to_radians();
            let (rx, ry) = (b.cos(), -b.sin());
            let (zx, zy) = rng.normal_pair();
            let x = rx * spec.offset_m + zx * spec.noise_m;
            let y = ry * spec.offset_m + zy * spec.noise_m;
            points.push(proj.to_latlon((x, y)));
        }
        legs.push(SidecarLeg {
            strand_id: l.strand.clone(),
            from_km: l.from_km,
            to_km: l.to_km,
            first_point: first,
            last_point: points.len() - 1,
        });
    }
    Ok(Drive { points, legs })
}

pub fn render_gpx(points: &[LatLon]) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str(
        "<gpx version=\"1.1\" creator=\"rastpipe\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n",
    );
    for (i, p) in points.iter().enumerate() {
        let t = format_rfc3339(START_TS + i as i64 * i64::from(INTERVAL_S));
        let _ = writeln!(
            s,
            "  <wpt lat=\"{:.6}\" lon=\"{:.6}\"><time>{}</time></wpt>",
            p.lat, p.lon, t
        );
    }
    s.push_str("</gpx>\n");
    s
}

pub fn render_sidecar(name: &str, spec: &DriveSpec, drive: &Drive) -> Result<String> {
    let sc = Sidecar {
        name: name.to_string(),
        seed: spec.seed,
        speed_kmh: spec.speed_kmh,
        offset_m: spec.offset_m,
        noise_m: spec.noise_m,
        interval_s: INTERVAL_S,
        legs: drive
            .legs
            .iter()
            .map(|l| SidecarLeg {
                strand_id: l.strand_id.clone(),
                from_km: l.from_km,
                to_km: l.to_km,
                first_point: l.first_point,
                last_point: l.last_point,
            })
            .collect(),
        expect: spec.expect.clone(),
    };
    let mut text = serde_json::to_string_pretty(&sc)
        .map_err(|e| Error::Json("cannot serialize sidecar".into(), e))?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::distance::haversine_m;
    use crate::geo::local::project_onto_polyline;

    fn straight_north() -> StrandGeom {
        let points = vec![LatLon::new(50.0, 10.0), LatLon::new(50.2, 10.0)];
        let cum_km = cumulative_km(&points);
        StrandGeom {
            id: "A1-N-0".into(),
            length_km: cum_km[1],
            points,
            cum_km,
        }
    }

    fn spec(offset: f64, noise: f64) -> DriveSpec {
        DriveSpec {
            legs: vec![LegSpec {
                strand: "A1-N-0".into(),
                from_km: 1.0,
                to_km: 11.0,
            }],
            speed_kmh: 120.0,
            offset_m: offset,
            noise_m: noise,
            seed: 5,
            expect: "matched".into(),
        }
    }

    #[test]
    fn one_hertz_spacing_and_count() {
        let s = vec![straight_north()];
        let d = generate(&spec(0.0, 0.0), &s).unwrap();
        // 10 km at 33.33 m/s = 300 s -> 301 points.
        assert_eq!(d.points.len(), 301);
        assert_eq!(d.legs[0].first_point, 0);
        assert_eq!(d.legs[0].last_point, 300);
        let gap = haversine_m(d.points[0], d.points[1]);
        assert!((gap - 120.0 / 3.6).abs() < 0.01, "{gap}");
    }

    #[test]
    fn offset_sign_is_right_of_travel() {
        let s = vec![straight_north()];
        let d = generate(&spec(120.0, 0.0), &s).unwrap();
        let cum = &s[0].cum_km;
        let pr = project_onto_polyline(&s[0].points, cum, d.points[10]).unwrap();
        assert!((pr.d - 120.0).abs() < 0.5, "{}", pr.d); // east of a northbound strand
        let d = generate(&spec(-50.0, 0.0), &s).unwrap();
        let pr = project_onto_polyline(&s[0].points, cum, d.points[10]).unwrap();
        assert!((pr.d + 50.0).abs() < 0.5, "{}", pr.d);
    }

    #[test]
    fn noise_is_seeded_and_scaled() {
        let s = vec![straight_north()];
        let a = generate(&spec(0.0, 8.0), &s).unwrap();
        let b = generate(&spec(0.0, 8.0), &s).unwrap();
        assert_eq!(render_gpx(&a.points), render_gpx(&b.points));
        let mut other = spec(0.0, 8.0);
        other.seed = 6;
        assert_ne!(
            render_gpx(&a.points),
            render_gpx(&generate(&other, &s).unwrap().points)
        );
        let n = a.points.len() as f64;
        let ds: Vec<f64> = a
            .points
            .iter()
            .map(|p| {
                project_onto_polyline(&s[0].points, &s[0].cum_km, *p)
                    .unwrap()
                    .d
            })
            .collect();
        let mean = ds.iter().sum::<f64>() / n;
        let sd = (ds.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
        assert!(mean.abs() < 2.0 && (sd - 8.0).abs() < 1.5, "{mean} {sd}");
    }

    #[test]
    fn legs_are_concatenated_and_arguments_validated() {
        let s = vec![straight_north()];
        let mut sp = spec(0.0, 0.0);
        sp.legs.push(LegSpec {
            strand: "A1-N-0".into(),
            from_km: 20.0,
            to_km: 21.0,
        });
        let d = generate(&sp, &s).unwrap();
        assert_eq!(d.legs[1].first_point, 301);
        assert_eq!(d.legs[1].last_point, d.points.len() - 1);
        let bad = |f: &dyn Fn(&mut DriveSpec)| {
            let mut x = spec(0.0, 0.0);
            f(&mut x);
            validate_spec(&x, &s).is_err()
        };
        assert!(bad(&|x| x.legs[0].strand = "nope".into()));
        assert!(bad(&|x| x.legs[0].from_km = -1.0));
        assert!(bad(&|x| x.legs[0].to_km = 1.0));
        assert!(bad(&|x| x.legs[0].to_km = 999.0));
        assert!(bad(&|x| x.speed_kmh = 0.0));
        assert!(bad(&|x| x.expect = "x".into()));
    }

    #[test]
    fn gpx_format() {
        let g = render_gpx(&[LatLon::new(50.1, 10.2), LatLon::new(50.1, 10.2)]);
        assert!(g.contains(
            "<wpt lat=\"50.100000\" lon=\"10.200000\"><time>2026-01-01T00:00:00Z</time></wpt>"
        ));
        assert!(g.contains("<time>2026-01-01T00:00:01Z</time>"));
        assert!(g.starts_with("<?xml"));
    }
}
