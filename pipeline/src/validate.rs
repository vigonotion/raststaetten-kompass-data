//! `rastpipe validate`: checks a dataset against docs/schema.md §2–3.

use crate::error::{Error, Result};
use crate::geo::distance::{bearing_deg, towards};
use crate::geo::local::cumulative_km;
use crate::geo::polyline;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const TOWARDS: [&str; 8] = ["N", "NO", "O", "SO", "S", "SW", "W", "NW"];

fn is_rounded(x: f64, decimals: i32) -> bool {
    let f = 10f64.powi(decimals);
    ((x * f).round() / f - x).abs() < 1e-9
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn canonical_number(s: &str) -> bool {
    all_digits(s) && (s == "0" || !s.starts_with('0'))
}

fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let (y, m, d) = (&s[0..4], &s[5..7], &s[8..10]);
    if !all_digits(y) || !all_digits(m) || !all_digits(d) {
        return false;
    }
    let (m, d): (u32, u32) = (m.parse().unwrap_or(0), d.parse().unwrap_or(0));
    (1..=12).contains(&m) && (1..=31).contains(&d)
}

fn valid_rfc3339_utc(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 20 || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| all_digits(&s[r]);
    if !(valid_date(&s[0..10]) && digits(11..13) && digits(14..16) && digits(17..19)) {
        return false;
    }
    let h: u32 = s[11..13].parse().unwrap_or(99);
    let mi: u32 = s[14..16].parse().unwrap_or(99);
    let se: u32 = s[17..19].parse().unwrap_or(99);
    h < 24 && mi < 60 && se < 61
}

/// Parses `A<digits>-<towards>-<n>`; returns (number, towards, n).
fn parse_strand_id(id: &str) -> Option<(&str, &str, &str)> {
    let mut it = id.split('-');
    let (a, t, n) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() {
        return None;
    }
    let num = a.strip_prefix('A')?;
    (canonical_number(num) && TOWARDS.contains(&t) && canonical_number(n)).then_some((num, t, n))
}

struct Ctx {
    errors: Vec<String>,
}

impl Ctx {
    fn err(&mut self, m: String) {
        self.errors.push(m);
    }
}

fn get_str<'a>(o: &'a Map<String, Value>, k: &str, at: &str, c: &mut Ctx) -> Option<&'a str> {
    match o.get(k) {
        Some(Value::String(s)) => Some(s),
        Some(_) => {
            c.err(format!("{at}: `{k}` must be a string"));
            None
        }
        None => {
            c.err(format!("{at}: missing `{k}`"));
            None
        }
    }
}

fn get_num(o: &Map<String, Value>, k: &str, at: &str, c: &mut Ctx) -> Option<f64> {
    match o.get(k) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(_) => {
            c.err(format!("{at}: `{k}` must be a number"));
            None
        }
        None => {
            c.err(format!("{at}: missing `{k}`"));
            None
        }
    }
}

fn get_opt_str<'a>(
    o: &'a Map<String, Value>,
    k: &str,
    at: &str,
    c: &mut Ctx,
) -> Option<Option<&'a str>> {
    match o.get(k) {
        Some(Value::String(s)) => Some(Some(s)),
        Some(Value::Null) => Some(None),
        Some(_) => {
            c.err(format!("{at}: `{k}` must be a string or null"));
            None
        }
        None => {
            c.err(format!("{at}: missing `{k}`"));
            None
        }
    }
}

struct StrandInfo {
    length_km: f64,
}

/// Validates a parsed dataset. `brand_ids` (curated order) enables the brand ID/order check.
pub fn validate_value(root: &Value, brand_ids: Option<&[String]>) -> Vec<String> {
    let mut c = Ctx { errors: Vec::new() };
    let Some(root) = root.as_object() else {
        return vec!["top level must be an object".into()];
    };

    match root.get("schemaVersion") {
        Some(Value::Number(n)) if n.as_i64() == Some(1) => {}
        _ => c.err("schemaVersion must be the integer 1".into()),
    }
    if let Some(v) = get_str(root, "dataVersion", "dataset", &mut c) {
        if !valid_date(v) {
            c.err(format!("dataVersion `{v}` is not YYYY-MM-DD"));
        }
    }
    if let Some(v) = get_str(root, "generatedAt", "dataset", &mut c) {
        if !valid_rfc3339_utc(v) {
            c.err(format!(
                "generatedAt `{v}` is not RFC 3339 UTC with Z (second precision)"
            ));
        }
    }
    if let Some(v) = get_str(root, "source", "dataset", &mut c) {
        if !v.contains("© OpenStreetMap contributors") || !v.contains("ODbL") {
            c.err("source must contain `© OpenStreetMap contributors` and `ODbL`".into());
        }
    }

    let strands = array(root, "strands", &mut c);
    let junctions = array(root, "junctions", &mut c);
    let facilities = array(root, "facilities", &mut c);

    // --- strands ---
    let mut infos: HashMap<String, StrandInfo> = HashMap::new();
    let mut groups: BTreeMap<(String, String), Vec<(u64, f64)>> = BTreeMap::new();
    let mut prev_id: Option<String> = None;
    for (i, s) in strands.iter().enumerate() {
        let Some(o) = s.as_object() else {
            c.err(format!("strands[{i}]: must be an object"));
            continue;
        };
        let id = get_str(o, "id", &format!("strands[{i}]"), &mut c)
            .unwrap_or("")
            .to_string();
        let at = format!("strand `{id}`");
        if let Some(p) = &prev_id {
            if p.as_bytes() >= id.as_bytes() {
                c.err(format!(
                    "{at}: strands not sorted by id in byte order (after `{p}`) or duplicate"
                ));
            }
        }
        prev_id = Some(id.clone());
        let parsed = parse_strand_id(&id);
        if parsed.is_none() {
            c.err(format!(
                "{at}: id does not match ^A[0-9]+-(N|NO|O|SO|S|SW|W|NW)-[0-9]+$"
            ));
        }
        let ref_ = get_str(o, "ref", &at, &mut c);
        if let Some(r) = ref_ {
            match r.strip_prefix("A ") {
                Some(n) if canonical_number(n) => {
                    if let Some((num, _, _)) = parsed {
                        if num != n {
                            c.err(format!("{at}: id number does not match ref `{r}`"));
                        }
                    }
                }
                _ => c.err(format!("{at}: ref `{r}` is not normalized `A <number>`")),
            }
        }
        let tw = get_str(o, "towards", &at, &mut c);
        if let Some(t) = tw {
            if !TOWARDS.contains(&t) {
                c.err(format!("{at}: invalid towards `{t}`"));
            } else if let Some((_, pt, _)) = parsed {
                if pt != t {
                    c.err(format!("{at}: id direction does not match towards `{t}`"));
                }
            }
        }
        let poly = get_str(o, "polyline", &at, &mut c);
        let length = get_num(o, "lengthKm", &at, &mut c);
        if let (Some(poly), Some(length)) = (poly, length) {
            match polyline::decode(poly) {
                Err(e) => c.err(format!("{at}: polyline invalid: {e}")),
                Ok(pts) if pts.len() < 2 => {
                    c.err(format!("{at}: polyline has fewer than 2 points"))
                }
                Ok(pts) => {
                    let cum = cumulative_km(&pts);
                    let total = *cum.last().unwrap_or(&0.0);
                    let rounded = (total * 1000.0).round() / 1000.0;
                    if rounded != length {
                        c.err(format!("{at}: lengthKm {length} != recomputed {rounded}"));
                    }
                    if let Some(t) = tw {
                        let sector = towards(bearing_deg(pts[0], pts[pts.len() - 1]));
                        if sector != t {
                            c.err(format!("{at}: towards `{t}` != bearing sector `{sector}` of first->last point"));
                        }
                    }
                    for (k, p) in pts.iter().enumerate() {
                        if !(-90.0..=90.0).contains(&p.lat) || !(-180.0..=180.0).contains(&p.lon) {
                            c.err(format!("{at}: point {k} out of WGS84 range"));
                            break;
                        }
                    }
                }
            }
            if !is_rounded(length, 3) {
                c.err(format!("{at}: lengthKm {length} not rounded to 3 decimals"));
            }
            if let Some((num, t, n)) = parsed {
                groups
                    .entry((num.to_string(), t.to_string()))
                    .or_default()
                    .push((n.parse().unwrap_or(u64::MAX), length));
            }
            infos.insert(id.clone(), StrandInfo { length_km: length });
        }
    }
    for ((num, t), list) in &groups {
        let mut ns: Vec<u64> = list.iter().map(|x| x.0).collect();
        ns.sort_unstable();
        if ns.iter().enumerate().any(|(k, n)| *n != k as u64) {
            c.err(format!(
                "strands A{num}-{t}-*: numbering is not 0..{}",
                ns.len().saturating_sub(1)
            ));
        }
        let mut by_n = list.clone();
        by_n.sort_by_key(|x| x.0);
        if by_n.windows(2).any(|w| w[0].1 < w[1].1) {
            c.err(format!(
                "strands A{num}-{t}-*: numbering is not by descending lengthKm"
            ));
        }
    }

    // --- junctions ---
    let mut prev: Option<(String, f64)> = None;
    for (i, j) in junctions.iter().enumerate() {
        let at = format!("junctions[{i}]");
        let Some(o) = j.as_object() else {
            c.err(format!("{at}: must be an object"));
            continue;
        };
        let sid = get_str(o, "strandId", &at, &mut c)
            .unwrap_or("")
            .to_string();
        let km = get_num(o, "km", &at, &mut c);
        match infos.get(&sid) {
            None => c.err(format!("{at}: unknown strandId `{sid}`")),
            Some(info) => {
                if let Some(km) = km {
                    if km < 0.0 || km > info.length_km {
                        c.err(format!("{at}: km {km} outside 0..{}", info.length_km));
                    }
                }
            }
        }
        if let Some(km) = km {
            if !is_rounded(km, 3) {
                c.err(format!("{at}: km {km} not rounded to 3 decimals"));
            }
            if let Some((ps, pk)) = &prev {
                let ord = ps.as_bytes().cmp(sid.as_bytes()).then(pk.total_cmp(&km));
                if ord == std::cmp::Ordering::Greater {
                    c.err(format!("{at}: not sorted by strandId, km"));
                }
            }
            prev = Some((sid.clone(), km));
        }
        let r = get_opt_str(o, "ref", &at, &mut c);
        let n = get_opt_str(o, "name", &at, &mut c);
        for (k, v) in [("ref", r), ("name", n)] {
            if let Some(Some(s)) = v {
                if s.trim() != s || s.is_empty() {
                    c.err(format!(
                        "{at}: `{k}` must be trimmed and non-empty (or null)"
                    ));
                }
            }
        }
        if r == Some(None) && n == Some(None) {
            c.err(format!("{at}: neither ref nor name set"));
        }
    }

    // --- facilities ---
    let mut ids: HashSet<String> = HashSet::new();
    let mut base_counts: HashMap<String, usize> = HashMap::new();
    let mut unsuffixed: Vec<String> = Vec::new();
    let mut prev: Option<(String, f64, String)> = None;
    for (i, f) in facilities.iter().enumerate() {
        let Some(o) = f.as_object() else {
            c.err(format!("facilities[{i}]: must be an object"));
            continue;
        };
        let id = get_str(o, "id", &format!("facilities[{i}]"), &mut c)
            .unwrap_or("")
            .to_string();
        let at = format!("facility `{id}`");
        if !ids.insert(id.clone()) {
            c.err(format!("{at}: duplicate id"));
        }
        let (base, suffix) = match id.split_once('@') {
            Some((b, s)) => (b, Some(s)),
            None => (id.as_str(), None),
        };
        let base_ok = base
            .strip_prefix("osm-")
            .and_then(|r| r.split_once('-'))
            .is_some_and(|(t, n)| ["node", "way", "relation"].contains(&t) && all_digits(n));
        if !base_ok {
            c.err(format!(
                "{at}: id must be osm-<node|way|relation>-<osmId>[@<strandId>]"
            ));
        }
        *base_counts.entry(base.to_string()).or_default() += 1;
        let sid = get_str(o, "strandId", &at, &mut c)
            .unwrap_or("")
            .to_string();
        if let Some(s) = suffix {
            if s != sid {
                c.err(format!("{at}: id suffix `@{s}` != strandId `{sid}`"));
            }
        }
        let kind = get_str(o, "kind", &at, &mut c);
        match kind {
            Some("rastanlage") => {
                if suffix.is_none() {
                    unsuffixed.push(base.to_string());
                }
            }
            Some("autohof") => {
                if suffix.is_none() {
                    c.err(format!("{at}: autohof ids need the @<strandId> suffix"));
                }
            }
            Some(k) => c.err(format!("{at}: invalid kind `{k}`")),
            None => {}
        }
        get_opt_str(o, "name", &at, &mut c);
        let km = get_num(o, "km", &at, &mut c);
        match infos.get(&sid) {
            None => c.err(format!("{at}: unknown strandId `{sid}`")),
            Some(info) => {
                if let Some(km) = km {
                    if km < 0.0 || km > info.length_km {
                        c.err(format!("{at}: km {km} outside 0..{}", info.length_km));
                    }
                }
            }
        }
        if let Some(km) = km {
            if !is_rounded(km, 3) {
                c.err(format!("{at}: km {km} not rounded to 3 decimals"));
            }
            if let Some((ps, pk, pid)) = &prev {
                let ord = ps
                    .as_bytes()
                    .cmp(sid.as_bytes())
                    .then(pk.total_cmp(&km))
                    .then(pid.as_bytes().cmp(id.as_bytes()));
                if ord == std::cmp::Ordering::Greater {
                    c.err(format!("{at}: not sorted by strandId, km, id"));
                }
            }
            prev = Some((sid.clone(), km, id.clone()));
        }
        for (k, lim) in [("lat", 90.0), ("lon", 180.0)] {
            if let Some(v) = get_num(o, k, &at, &mut c) {
                if v.abs() > lim {
                    c.err(format!("{at}: {k} {v} out of range"));
                }
                if !is_rounded(v, 5) {
                    c.err(format!("{at}: {k} {v} not rounded to 5 decimals"));
                }
            }
        }
        let exit = get_opt_str(o, "exit", &at, &mut c);
        let detour = get_num(o, "detourKm", &at, &mut c);
        match kind {
            Some("rastanlage") => {
                if exit != Some(None) {
                    c.err(format!("{at}: rastanlage exit must be null"));
                }
                if detour != Some(0.0) {
                    c.err(format!("{at}: rastanlage detourKm must be 0"));
                }
            }
            Some("autohof") => {
                if !matches!(exit, Some(Some(s)) if !s.trim().is_empty()) {
                    c.err(format!("{at}: autohof exit must be a non-empty string"));
                }
                if let Some(d) = detour {
                    if d <= 0.0 || !is_rounded(d, 1) {
                        c.err(format!("{at}: detourKm {d} must be > 0 with 1 decimal"));
                    }
                }
            }
            _ => {}
        }
        match o.get("brands") {
            Some(Value::Array(b)) => {
                let names: Vec<&str> = b.iter().filter_map(Value::as_str).collect();
                if names.len() != b.len() {
                    c.err(format!("{at}: brands must be strings"));
                }
                let mut seen = HashSet::new();
                if !names.iter().all(|n| seen.insert(*n)) {
                    c.err(format!("{at}: brands contain duplicates"));
                }
                if let Some(known) = brand_ids {
                    let mut last = None;
                    for n in &names {
                        match known.iter().position(|k| k == n) {
                            None => c.err(format!("{at}: unknown brand `{n}`")),
                            Some(p) => {
                                if last.is_some_and(|l| l > p) {
                                    c.err(format!("{at}: brands not in brands.json order"));
                                }
                                last = Some(p);
                            }
                        }
                    }
                }
            }
            _ => c.err(format!("{at}: `brands` must be an array of strings")),
        }
        for k in ["shower", "fastCharger", "open24h"] {
            if !matches!(o.get(k), Some(Value::Bool(_))) {
                c.err(format!("{at}: `{k}` must be a boolean"));
            }
        }
    }
    for b in unsuffixed {
        if base_counts.get(&b).copied().unwrap_or(0) > 1 {
            c.err(format!("facility `{b}`: emitted on several strands, every entry needs the @<strandId> suffix"));
        }
    }
    c.errors
}

fn array<'a>(root: &'a Map<String, Value>, key: &str, c: &mut Ctx) -> &'a [Value] {
    match root.get(key) {
        Some(Value::Array(a)) => a,
        _ => {
            c.err(format!("`{key}` must be an array"));
            &[]
        }
    }
}

/// Reads and validates a dataset file. Returns the number of strands/junctions/facilities.
pub fn validate_file(path: &str, brands_path: Option<&str>) -> Result<(usize, usize, usize)> {
    let text =
        std::fs::read_to_string(path).map_err(|e| Error::Io(format!("cannot read {path}"), e))?;
    let v: Value =
        serde_json::from_str(&text).map_err(|e| Error::Json(format!("cannot parse {path}"), e))?;
    let brand_ids: Option<Vec<String>> = match brands_path {
        Some(p) => Some(crate::brands::load(p)?.into_iter().map(|b| b.id).collect()),
        None => None,
    };
    let errors = validate_value(&v, brand_ids.as_deref());
    if !errors.is_empty() {
        let total = errors.len();
        let mut shown: Vec<String> = errors.into_iter().take(100).collect();
        if total > 100 {
            shown.push(format!("... and {} more", total - 100));
        }
        return Err(Error::Validation(shown));
    }
    let n = |k: &str| v[k].as_array().map_or(0, Vec::len);
    Ok((n("strands"), n("junctions"), n("facilities")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Value {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/example-dataset.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn example_dataset_passes() {
        let errs = validate_value(&example(), None);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn example_dataset_passes_with_brands() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/brands.json");
        let ids: Vec<String> = crate::brands::load(path)
            .unwrap()
            .into_iter()
            .map(|b| b.id)
            .collect();
        let errs = validate_value(&example(), Some(&ids));
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn detects_problems() {
        let mut v = example();
        v["strands"][0]["lengthKm"] = Value::from(99.5);
        v["junctions"][0]["strandId"] = Value::from("A9-N-0");
        v["facilities"][1]["id"] = Value::from("osm-way-1x");
        v["schemaVersion"] = Value::from(2);
        v["strands"].as_array_mut().unwrap().swap(0, 1);
        let errs = validate_value(&v, None);
        assert!(errs.iter().any(|e| e.contains("lengthKm")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("unknown strandId")));
        assert!(errs.iter().any(|e| e.contains("osm-<node|way|relation>")));
        assert!(errs.iter().any(|e| e.contains("schemaVersion")));
        assert!(errs.iter().any(|e| e.contains("not sorted")));
    }

    #[test]
    fn id_parsing() {
        assert!(parse_strand_id("A1-SW-0").is_some());
        assert!(parse_strand_id("A10-NO-12").is_some());
        assert!(parse_strand_id("A1-X-0").is_none());
        assert!(parse_strand_id("A01-N-0").is_none());
        assert!(parse_strand_id("A1-N-0-1").is_none());
        assert!(parse_strand_id("B1-N-0").is_none());
    }
}
