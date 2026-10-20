//! rastpipe: offline OSM pipeline for Raststaetten-Kompass.

mod amenities;
mod autohof;
mod brands;
mod error;
mod facilities;
mod geo;
mod gpx;
mod index;
mod invariant;
mod osm;
mod output;
mod params;
mod prng;
mod report;
mod strands;
mod validate;

use error::{Error, Result};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

const USAGE: &str = "\
rastpipe: OSM motorway service area pipeline

USAGE:
  rastpipe build    --input <pbf> [--input <pbf>...] --brands <json> --out <dir> [--strict] [--no-autohoefe]
                    [--dataset-url <url>]   (manifest url; default dataset.json.gz; `{dataVersion}` is substituted;
                                             relative path without `..` or https:// URL)
  rastpipe validate --dataset <json> [--brands <json>]
  rastpipe gpx      --dataset <json> --strand <id> --from-km <f> --to-km <f> --speed-kmh <f>
                    [--strand <id> --from-km <f> --to-km <f>]...   (repeatable legs, in order)
                    [--offset-m <f>] [--noise-m <f>] [--seed <n>]
                    [--expect matched|notOnMotorway|tooSlow] --out <gpx>
                    (also writes <gpx without extension>.json next to it)
";

/// Parsed command line options: repeated value flags and boolean switches.
struct Flags {
    values: HashMap<String, Vec<String>>,
    switches: Vec<String>,
}

impl Flags {
    fn parse(args: &[String], value_flags: &[&str], switch_flags: &[&str]) -> Result<Flags> {
        let mut f = Flags {
            values: HashMap::new(),
            switches: Vec::new(),
        };
        let mut i = 0;
        while i < args.len() {
            let a = args[i].as_str();
            if switch_flags.contains(&a) {
                f.switches.push(a.to_string());
            } else if value_flags.contains(&a) {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| Error::Usage(format!("missing value for {a}")))?;
                f.values.entry(a.to_string()).or_default().push(v.clone());
            } else {
                return Err(Error::Usage(format!("unknown argument: {a}")));
            }
            i += 1;
        }
        Ok(f)
    }

    fn one(&self, name: &str) -> Result<Option<&str>> {
        match self.values.get(name).map(Vec::as_slice) {
            None => Ok(None),
            Some([v]) => Ok(Some(v)),
            Some(_) => Err(Error::Usage(format!("{name} given more than once"))),
        }
    }

    fn require(&self, name: &str) -> Result<&str> {
        self.one(name)?
            .ok_or_else(|| Error::Usage(format!("missing required argument {name}")))
    }

    fn has(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
}

fn parse_f64(flags: &Flags, name: &str, required: bool) -> Result<Option<f64>> {
    let v = if required {
        Some(flags.require(name)?)
    } else {
        flags.one(name)?
    };
    v.map(|s| {
        s.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .ok_or_else(|| Error::Usage(format!("{name} expects a number, got `{s}`")))
    })
    .transpose()
}

fn cmd_build(args: &[String]) -> Result<()> {
    let flags = Flags::parse(
        args,
        &["--input", "--brands", "--out", "--dataset-url"],
        &["--strict", "--no-autohoefe"],
    )?;
    let inputs: Vec<String> = flags.values.get("--input").cloned().unwrap_or_default();
    if inputs.is_empty() {
        return Err(Error::Usage("missing required argument --input".into()));
    }
    let brands_path = flags.require("--brands")?.to_string();
    let out_dir = flags.require("--out")?.to_string();
    let strict = flags.has("--strict");
    let no_autohoefe = flags.has("--no-autohoefe");

    let started = Instant::now();
    let brands = brands::load(&brands_path)?;
    eprintln!("brands: {} entries valid", brands.len());

    // dataVersion / generatedAt: latest replication timestamp of the inputs, else build time.
    let mut ts: Option<i64> = None;
    for p in &inputs {
        if let Some(t) = osm::read::read_header_timestamp(p)? {
            ts = Some(ts.map_or(t, |x| x.max(t)));
        }
    }
    let (ts, ts_source) = match ts {
        Some(t) => (
            t,
            "PBF header osmosis_replication_timestamp (latest of all inputs)",
        ),
        None => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            (now, "build date (no replication timestamp in PBF header)")
        }
    };
    let data_version = output::format_date(ts);
    let dataset_url = output::resolve_dataset_url(
        flags
            .one("--dataset-url")?
            .unwrap_or(output::DEFAULT_DATASET_URL),
        &data_version,
    )?;
    let generated_at = output::format_rfc3339(ts);

    let (model, mut passes) = osm::read::read_all(&inputs)?;
    for p in &passes {
        eprintln!(
            "{}: {:.2} s, peak RSS {:?} kB",
            p.name, p.seconds, p.peak_rss_kb
        );
    }

    let set = strands::build(&model);
    eprintln!(
        "{} strands, {} junctions, {} remnants",
        set.strands.len(),
        set.junctions.len(),
        set.remnants.len()
    );

    // Rastanlagen: assignment, corridor, brands, amenities (heuristics §4-§6).
    let t_assign = Instant::now();
    let index = index::StrandIndex::new(&set.strands);
    let matcher = brands::Matcher::new(&brands);
    let (all_sources, skipped_sources) = facilities::collect_sources(&model);
    let total_objects = all_sources.len();
    let exclude: std::collections::HashSet<facilities::ObjRef> =
        all_sources.iter().map(|s| s.obj).collect();
    let pois = facilities::collect_pois(&model, &exclude);
    let facilities::Prepared {
        sources,
        deferred,
        merged,
    } = facilities::prepare(all_sources, &set.strands, &index);
    let ctx = facilities::Context {
        strands: &set.strands,
        index: &index,
        matcher: &matcher,
        brands: &brands,
    };
    let assignment = facilities::assign(&sources, &pois, &ctx);
    let entries = facilities::entries(&assignment, &sources, &set.strands, &brands);
    passes.push(osm::model::PassStats {
        name: "Facility assignment (strands, corridor, brands, amenities)".into(),
        seconds: t_assign.elapsed().as_secs_f64(),
        peak_rss_kb: osm::read::peak_rss_kb(),
    });
    eprintln!(
        "{} facility objects ({} Autohof-like deferred, {} merged duplicates): {} assigned ({} entries), {} unassigned, {} POIs excluded by the invariant filter",
        total_objects,
        deferred.len(),
        merged.iter().map(|g| g.dropped.len()).sum::<usize>(),
        assignment.placed.len(),
        entries.len(),
        assignment.unassigned.len(),
        assignment.excluded.len()
    );

    // Invariant, stage 2: independent assertion over the final assignments.
    let t_check = Instant::now();
    let mut to_check: Vec<invariant::Assignment<'_>> = Vec::new();
    for p in &assignment.placed {
        for &pi in &p.pois {
            to_check.push(invariant::Assignment {
                facility: sources[p.src].obj.base_id(),
                poi: pois[pi].obj.base_id(),
                strand: &set.strands[p.strand],
                centroid_km: p.proj.km,
                poi_lat: pois[pi].pos.lat,
                poi_lon: pois[pi].pos.lon,
            });
        }
    }
    let violations = invariant::check(&to_check, &set.strands);
    passes.push(osm::model::PassStats {
        name: format!("Invariant assertion ({} assigned POIs)", to_check.len()),
        seconds: t_check.elapsed().as_secs_f64(),
        peak_rss_kb: osm::read::peak_rss_kb(),
    });
    eprintln!("invariant violations: {}", violations.len());
    if strict && !violations.is_empty() {
        for v in &violations {
            eprintln!(
                "violation: facility {} / POI {}: {}",
                v.facility, v.poi, v.reason
            );
        }
        return Err(Error::Data(format!(
            "{} invariant violation(s) in --strict mode; no output written",
            violations.len()
        )));
    }

    // Autohoefe (heuristics §7). They are exempt from the invariant assertion above: they use a
    // circle around the fuel station, not a corridor along a strand.
    let autohof_result = if no_autohoefe {
        eprintln!("Autohoefe skipped (--no-autohoefe)");
        None
    } else {
        let t_auto = Instant::now();
        let truck_parkings = facilities::collect_truck_parkings(&model);
        let rastanlagen: Vec<(facilities::ObjRef, geo::LatLon)> = assignment
            .placed
            .iter()
            .map(|p| (sources[p.src].obj, sources[p.src].centroid))
            .collect();
        let assigned: std::collections::HashSet<usize> = assignment
            .placed
            .iter()
            .flat_map(|p| p.pois.iter().copied())
            .collect();
        let invariant_excluded: std::collections::HashSet<usize> =
            assignment.excluded.iter().map(|e| e.poi).collect();
        let r = autohof::build(&autohof::AutohofInput {
            strands: &set.strands,
            junctions: &set.junctions,
            pois: &pois,
            assigned: &assigned,
            invariant_excluded: &invariant_excluded,
            rast_sources: &sources,
            rastanlagen: &rastanlagen,
            deferred: &deferred,
            matcher: &matcher,
            brands: &brands,
            truck_parkings: &truck_parkings,
        });
        passes.push(osm::model::PassStats {
            name: "Autohoefe (clusters, candidates, entries)".into(),
            seconds: t_auto.elapsed().as_secs_f64(),
            peak_rss_kb: osm::read::peak_rss_kb(),
        });
        eprintln!(
            "Autohoefe: {} clusters, {} candidates, {} qualified, {} entries ({} distinct)",
            r.counts.clusters,
            r.counts.candidates,
            r.counts.qualified,
            r.counts.entries,
            r.counts.distinct
        );
        Some(r)
    };
    let mut all_entries: Vec<facilities::FacilityEntry> = entries.clone();
    if let Some(r) = &autohof_result {
        all_entries.extend(r.entries.iter().cloned());
    }
    all_entries.sort_by(|a, b| {
        a.strand_id
            .as_bytes()
            .cmp(b.strand_id.as_bytes())
            .then(a.km.total_cmp(&b.km))
            .then(a.id.as_bytes().cmp(b.id.as_bytes()))
    });

    let names: Vec<String> = inputs
        .iter()
        .map(|p| {
            Path::new(p)
                .file_name()
                .map_or(p.clone(), |n| n.to_string_lossy().into_owned())
        })
        .collect();
    let source = format!(
        "Geofabrik {}, © OpenStreetMap contributors, ODbL 1.0 (https://opendatacommons.org/licenses/odbl/1-0/)",
        names.join(", ")
    );
    let json = output::build_dataset_json(
        &data_version,
        &generated_at,
        &source,
        &set.strands,
        &set.junctions,
        &all_entries,
    )?;
    let out = Path::new(&out_dir);
    let manifest = output::write_files(out, &json, &data_version, &dataset_url)?;

    let collected = vec![
        (
            "Motorway ways with A ref".to_string(),
            model.motorway_ways.len(),
        ),
        ("Node coordinates loaded".to_string(), model.coords.len()),
        ("Junction nodes".to_string(), model.junction_nodes.len()),
        (
            "highway=services closed ways".to_string(),
            model.service_ways.len(),
        ),
        (
            "highway=services multipolygon relations".to_string(),
            model.service_relations.len(),
        ),
        (
            "Outer member ways of service relations".to_string(),
            model.member_ways.len(),
        ),
        (
            "highway=services nodes".to_string(),
            model.service_nodes.len(),
        ),
        ("POI nodes".to_string(), model.poi_nodes.len()),
        ("POI closed ways".to_string(), model.poi_ways.len()),
    ];
    let rep = report::render(&report::ReportInput {
        inputs: &names,
        data_version: &data_version,
        data_version_source: ts_source,
        generated_at: &generated_at,
        strands: &set.strands,
        junction_count: set.junctions.len(),
        remnants: &set.remnants,
        passes: &passes,
        collected,
        duplicates: model.duplicates,
        total_seconds: started.elapsed().as_secs_f64(),
        peak_rss_kb: osm::read::peak_rss_kb(),
        brands: &brands,
        sources: &sources,
        pois: &pois,
        assignment: &assignment,
        entries: &entries,
        violations: &violations,
        skipped_sources,
        total_objects,
        deferred: &deferred,
        merged: &merged,
        autohof: autohof_result.as_ref(),
    });
    let rp = out.join("report.md");
    std::fs::write(&rp, rep).map_err(|e| Error::Io(format!("cannot write {}", rp.display()), e))?;
    eprintln!(
        "wrote dataset.json ({} bytes), dataset.json.gz ({} bytes), manifest.json, report.md to {}",
        json.len(),
        manifest.size_bytes,
        out.display()
    );
    Ok(())
}

fn cmd_validate(args: &[String]) -> Result<()> {
    let flags = Flags::parse(args, &["--dataset", "--brands"], &[])?;
    let dataset = flags.require("--dataset")?;
    let (s, j, f) = validate::validate_file(dataset, flags.one("--brands")?)?;
    println!("OK: {dataset} is valid ({s} strands, {j} junctions, {f} facilities)");
    Ok(())
}

fn cmd_gpx(args: &[String]) -> Result<()> {
    let flags = Flags::parse(
        args,
        &[
            "--dataset",
            "--strand",
            "--from-km",
            "--to-km",
            "--speed-kmh",
            "--offset-m",
            "--noise-m",
            "--seed",
            "--expect",
            "--out",
        ],
        &[],
    )?;
    let dataset = flags.require("--dataset")?;
    let out = flags.require("--out")?;
    let speed_kmh = parse_f64(&flags, "--speed-kmh", true)?.unwrap_or(0.0);
    let offset_m = parse_f64(&flags, "--offset-m", false)?.unwrap_or(0.0);
    let noise_m = parse_f64(&flags, "--noise-m", false)?.unwrap_or(0.0);
    let seed = match flags.one("--seed")? {
        Some(s) => s
            .parse::<u64>()
            .map_err(|_| Error::Usage(format!("--seed expects an unsigned integer, got `{s}`")))?,
        None => 1,
    };
    let expect = flags.one("--expect")?.unwrap_or("matched").to_string();
    let list = |name: &str| flags.values.get(name).cloned().unwrap_or_default();
    let (strands_arg, from, to) = (list("--strand"), list("--from-km"), list("--to-km"));
    if strands_arg.is_empty() {
        return Err(Error::Usage("missing required argument --strand".into()));
    }
    if strands_arg.len() != from.len() || strands_arg.len() != to.len() {
        return Err(Error::Usage(
            "--strand, --from-km and --to-km must be given the same number of times".into(),
        ));
    }
    let num = |name: &str, s: &str| {
        s.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .ok_or_else(|| Error::Usage(format!("{name} expects a number, got `{s}`")))
    };
    let mut legs = Vec::new();
    for i in 0..strands_arg.len() {
        legs.push(gpx::LegSpec {
            strand: strands_arg[i].clone(),
            from_km: num("--from-km", &from[i])?,
            to_km: num("--to-km", &to[i])?,
        });
    }
    let spec = gpx::DriveSpec {
        legs,
        speed_kmh,
        offset_m,
        noise_m,
        seed,
        expect,
    };
    let strands = gpx::load_strands(dataset)?;
    let drive = gpx::generate(&spec, &strands)?;
    let out_path = Path::new(out);
    let stem = out_path
        .file_stem()
        .map_or_else(|| "drive".to_string(), |s| s.to_string_lossy().into_owned());
    let gpx_text = gpx::render_gpx(&drive.points);
    let sidecar = gpx::render_sidecar(&stem, &spec, &drive)?;
    let sidecar_path = out_path.with_extension("json");
    std::fs::write(out_path, gpx_text)
        .map_err(|e| Error::Io(format!("cannot write {}", out_path.display()), e))?;
    std::fs::write(&sidecar_path, sidecar)
        .map_err(|e| Error::Io(format!("cannot write {}", sidecar_path.display()), e))?;
    eprintln!(
        "wrote {} ({} points) and {}",
        out_path.display(),
        drive.points.len(),
        sidecar_path.display()
    );
    Ok(())
}

fn run(args: Vec<String>) -> Result<()> {
    match args.get(1).map(String::as_str) {
        Some("build") => cmd_build(&args[2..]),
        Some("validate") => cmd_validate(&args[2..]),
        Some("gpx") => cmd_gpx(&args[2..]),
        Some(other) => Err(Error::Usage(format!("unknown command: {other}"))),
        None => Err(Error::Usage("missing command".into())),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = run(args);
    if let Err(e) = result {
        eprintln!("error: {e}");
        if matches!(e, Error::Usage(_)) {
            eprintln!("\n{USAGE}");
        }
        std::process::exit(e.exit_code());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_autohoefe_is_a_switch() {
        let args: Vec<String> = ["--input", "a.pbf", "--no-autohoefe", "--strict"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let f = Flags::parse(&args, &["--input"], &["--strict", "--no-autohoefe"]).unwrap();
        assert!(f.has("--no-autohoefe") && f.has("--strict"));
        let f = Flags::parse(&args[..2], &["--input"], &["--strict", "--no-autohoefe"]).unwrap();
        assert!(!f.has("--no-autohoefe"));
    }
}
