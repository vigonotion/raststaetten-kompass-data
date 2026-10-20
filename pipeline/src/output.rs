//! Dataset, manifest and gzip writing (docs/schema.md §1–4).

use crate::error::{Error, Result};
use crate::facilities::FacilityEntry;
use crate::strands::{Junction, Strand};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;
use std::io::Write;
use std::path::Path;
use std::process::Command;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrandOut<'a> {
    pub id: &'a str,
    #[serde(rename = "ref")]
    pub ref_: &'a str,
    pub towards: &'a str,
    pub polyline: &'a str,
    pub length_km: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JunctionOut<'a> {
    pub strand_id: &'a str,
    pub km: f64,
    #[serde(rename = "ref")]
    pub ref_: Option<String>,
    pub name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FacilityOut<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub name: Option<&'a str>,
    pub strand_id: &'a str,
    pub km: f64,
    pub lat: f64,
    pub lon: f64,
    pub exit: Option<&'a str>,
    pub detour_km: f64,
    pub brands: &'a [String],
    pub shower: bool,
    pub fast_charger: bool,
    pub open24h: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetOut<'a> {
    pub schema_version: u32,
    pub data_version: &'a str,
    pub generated_at: &'a str,
    pub source: &'a str,
    pub strands: Vec<StrandOut<'a>>,
    pub junctions: Vec<JunctionOut<'a>>,
    pub facilities: Vec<FacilityOut<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub data_version: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

/// Civil date (year, month, day) from days since 1970-01-01 (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYY-MM-DD` (UTC) for a Unix timestamp in seconds.
pub fn format_date(ts: i64) -> String {
    let (y, m, d) = civil_from_days(ts.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// RFC 3339 UTC with `Z`, second precision.
pub fn format_rfc3339(ts: i64) -> String {
    let secs = ts.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        format_date(ts),
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

pub fn build_dataset_json(
    data_version: &str,
    generated_at: &str,
    source: &str,
    strands: &[Strand],
    junctions: &[Junction],
    facilities: &[FacilityEntry],
) -> Result<String> {
    let ds = DatasetOut {
        schema_version: 1,
        data_version,
        generated_at,
        source,
        strands: strands
            .iter()
            .map(|s| StrandOut {
                id: &s.id,
                ref_: &s.ref_,
                towards: s.towards,
                polyline: &s.polyline,
                length_km: s.length_km,
            })
            .collect(),
        junctions: junctions
            .iter()
            .map(|j| JunctionOut {
                strand_id: &j.strand_id,
                km: j.km,
                ref_: j
                    .ref_
                    .as_deref()
                    .map(crate::strands::display_list)
                    .filter(|v| !v.is_empty()),
                name: j
                    .name
                    .as_deref()
                    .map(crate::strands::display_list)
                    .filter(|v| !v.is_empty()),
            })
            .collect(),
        facilities: facilities
            .iter()
            .map(|f| FacilityOut {
                id: &f.id,
                kind: f.kind,
                name: f.name.as_deref(),
                strand_id: &f.strand_id,
                km: f.km,
                lat: f.lat,
                lon: f.lon,
                exit: f.exit.as_deref(),
                detour_km: f.detour_km,
                brands: &f.brands,
                shower: f.shower,
                fast_charger: f.fast_charger,
                open24h: f.open24h,
            })
            .collect(),
    };
    serde_json::to_string(&ds).map_err(|e| Error::Json("cannot serialize dataset".into(), e))
}

fn io_err(ctx: &str, path: &Path) -> impl FnOnce(std::io::Error) -> Error {
    let msg = format!("{ctx} {}", path.display());
    move |e| Error::Io(msg, e)
}

pub fn gzip_bytes(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::best());
    enc.write_all(data)
        .map_err(|e| Error::Io("gzip".into(), e))?;
    enc.finish().map_err(|e| Error::Io("gzip".into(), e))
}

/// SHA-256 via `shasum -a 256` (PLAN D-12, no crate).
pub fn sha256_of_file(path: &Path) -> Result<String> {
    let out = Command::new("shasum")
        .arg("-a")
        .arg("256")
        .arg(path)
        .output()
        .map_err(|e| Error::Io("cannot run shasum".into(), e))?;
    if !out.status.success() {
        return Err(Error::Command(format!(
            "shasum failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let hex = text.split_whitespace().next().unwrap_or("");
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Command(format!("unexpected shasum output: {text}")));
    }
    Ok(hex.to_ascii_lowercase())
}

/// Default manifest `url`: relative to the manifest itself (schema §4).
pub const DEFAULT_DATASET_URL: &str = "dataset.json.gz";

/// Resolves the `--dataset-url` value: replaces `{dataVersion}` and checks that the result is
/// either a relative path without `..` or an `https://` URL.
pub fn resolve_dataset_url(template: &str, data_version: &str) -> Result<String> {
    let url = template.replace("{dataVersion}", data_version);
    let bad = |why: &str| Error::Usage(format!("invalid --dataset-url `{url}`: {why}"));
    if url.is_empty() {
        return Err(bad("empty"));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad("contains whitespace or control characters"));
    }
    if let Some(rest) = url.strip_prefix("https://") {
        if rest.is_empty() || rest.starts_with('/') {
            return Err(bad("missing host"));
        }
        return Ok(url);
    }
    if url.contains("://") || url.starts_with('/') || url.contains('\\') {
        return Err(bad("must be a relative path or start with https://"));
    }
    if url.split('/').any(|seg| seg == "..") {
        return Err(bad("relative path must not contain `..`"));
    }
    Ok(url)
}

/// Writes `dataset.json`, `dataset.json.gz` and `manifest.json` into `out_dir`.
pub fn write_files(
    out_dir: &Path,
    dataset_json: &str,
    data_version: &str,
    url: &str,
) -> Result<Manifest> {
    std::fs::create_dir_all(out_dir).map_err(io_err("cannot create", out_dir))?;
    let json_path = out_dir.join("dataset.json");
    std::fs::write(&json_path, dataset_json.as_bytes())
        .map_err(io_err("cannot write", &json_path))?;
    let gz = gzip_bytes(dataset_json.as_bytes())?;
    let gz_path = out_dir.join("dataset.json.gz");
    std::fs::write(&gz_path, &gz).map_err(io_err("cannot write", &gz_path))?;
    let manifest = Manifest {
        schema_version: 1,
        data_version: data_version.to_string(),
        url: url.to_string(),
        sha256: sha256_of_file(&gz_path)?,
        size_bytes: gz.len() as u64,
    };
    let mut text = serde_json::to_string_pretty(&manifest)
        .map_err(|e| Error::Json("cannot serialize manifest".into(), e))?;
    text.push('\n');
    let mp = out_dir.join("manifest.json");
    std::fs::write(&mp, text).map_err(io_err("cannot write", &mp))?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_url_placeholder_and_validation() {
        let r = |t: &str| resolve_dataset_url(t, "2026-10-19");
        assert_eq!(r("dataset.json.gz").unwrap(), "dataset.json.gz");
        assert_eq!(
            r("https://x.org/releases/download/{dataVersion}/dataset.json.gz").unwrap(),
            "https://x.org/releases/download/2026-10-19/dataset.json.gz"
        );
        assert_eq!(r("{dataVersion}/d.gz").unwrap(), "2026-10-19/d.gz");
        assert!(r("../d.gz").is_err());
        assert!(r("a/../d.gz").is_err());
        assert!(r("/abs/d.gz").is_err());
        assert!(r("http://x.org/d.gz").is_err());
        assert!(r("ftp://x.org/d.gz").is_err());
        assert!(r("https://").is_err());
        assert!(r("").is_err());
        assert!(r("a b").is_err());
        assert!(matches!(r("../x"), Err(Error::Usage(_))));
    }

    #[test]
    fn dates() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_rfc3339(1_760_000_000), "2025-10-09T08:53:20Z");
        assert_eq!(format_date(951_782_400), "2000-02-29");
        assert_eq!(
            format_rfc3339(1_792_368_000 + 3 * 3600),
            "2026-10-19T03:00:00Z"
        );
    }

    #[test]
    fn gzip_round_trip() {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let data = b"hello hello hello hello";
        let gz = gzip_bytes(data).unwrap();
        assert_eq!(gz, gzip_bytes(data).unwrap());
        let mut out = Vec::new();
        GzDecoder::new(&gz[..]).read_to_end(&mut out).unwrap();
        assert_eq!(out, data);
    }
}
