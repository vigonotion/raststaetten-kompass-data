//! `report.md` (docs/heuristics.md §9).

use crate::autohof::{cluster_label, AutohofResult};
use crate::brands::Brand;
use crate::facilities::{Assignment, Deferred, FacilityEntry, MergeGroup, Poi, Source};
use crate::invariant::Violation;
use crate::osm::model::PassStats;
use crate::params::{SAMPLE_COUNT, SAMPLE_SEED};
use crate::prng::SplitMix64;
use crate::strands::{Remnant, Strand};
use std::collections::BTreeMap;
use std::fmt::Write;

pub struct ReportInput<'a> {
    pub inputs: &'a [String],
    pub data_version: &'a str,
    pub data_version_source: &'a str,
    pub generated_at: &'a str,
    pub strands: &'a [Strand],
    pub junction_count: usize,
    pub remnants: &'a [Remnant],
    pub passes: &'a [PassStats],
    /// Counts of collected objects, in display order.
    pub collected: Vec<(String, usize)>,
    pub duplicates: usize,
    pub total_seconds: f64,
    pub peak_rss_kb: Option<u64>,
    pub brands: &'a [Brand],
    pub sources: &'a [Source],
    pub pois: &'a [Poi<'a>],
    pub assignment: &'a Assignment,
    pub entries: &'a [FacilityEntry],
    pub violations: &'a [Violation],
    pub skipped_sources: usize,
    /// All `highway=services` objects before exclusion and merging.
    pub total_objects: usize,
    pub deferred: &'a [Deferred],
    pub merged: &'a [MergeGroup],
    /// `None` with `--no-autohoefe`.
    pub autohof: Option<&'a AutohofResult>,
}

/// Maximum rows of long tables (the full counts are always printed).
const MAX_ROWS: usize = 300;

fn mb(kb: u64) -> String {
    format!("{:.1} MB", kb as f64 / 1024.0)
}

/// Markdown table cell: no pipes or line breaks.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn name_of(tags: &crate::osm::model::Tags) -> String {
    tags.get("name")
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map_or("(no name)".to_string(), cell)
}

fn link(o: &crate::facilities::ObjRef) -> String {
    format!("[{}/{}]({})", o.typ.as_str(), o.id, o.url())
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn autohof_section(s: &mut String, r: &ReportInput<'_>) {
    let _ = writeln!(s, "\n## Autohöfe\n");
    let Some(x) = r.autohof else {
        let _ = writeln!(s, "Skipped (`--no-autohoefe`).");
        return;
    };
    let c = &x.counts;
    let _ = writeln!(
        s,
        "Invariant (§6.3): Autohöfe are exempt from the stage-2 assertion. They take brands and amenities from a circle of 150 m around the position, not from a corridor along a strand, and the assertion only covers Rastanlage POIs.\n"
    );
    let _ = writeln!(s, "- Junction clusters: {}", c.clusters);
    let _ = writeln!(
        s,
        "- Fuel stations: {} total; {} assigned to a Rastanlage, {} in the corridor of a Rastanlage but excluded by the invariant filter, {} inside a Rastanlage polygon, {} merged into a deferred services object (D-31)",
        c.fuel_total,
        c.fuel_assigned,
        c.fuel_invariant_excluded,
        c.fuel_in_rastanlage_polygon,
        c.fuel_merged_into_services
    );
    let _ = writeln!(
        s,
        "- Candidates within 2 km of a junction node: {} ({} deferred services objects, {} fuel stations)",
        c.candidates,
        c.deferred_candidates,
        c.candidates - c.deferred_candidates
    );
    let _ = writeln!(
        s,
        "- Qualified: {} (rules, may overlap: deferred services object {}, Autohof keyword {}, gastronomy POI within 150 m and truck parking within 300 m {})",
        c.qualified, c.q_deferred, c.q_keyword, c.q_gastro
    );
    let _ = writeln!(
        s,
        "- Emitted: {} entries, {} distinct Autohöfe, {} cluster picks",
        c.entries,
        c.distinct,
        x.placed.len()
    );
    let by_rule = |f: &dyn Fn(&crate::autohof::Reasons) -> bool| {
        x.placed
            .iter()
            .filter(|p| f(&p.reasons))
            .map(|p| x.cands[p.cand].obj)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    let _ = writeln!(
        s,
        "- Emitted Autohöfe by rule (distinct): deferred services object {}, Autohof keyword without deferred {}, gastronomy + truck parking only (within 1 km of the junction) {}",
        by_rule(&|r| r.deferred),
        by_rule(&|r| r.keyword && !r.deferred),
        by_rule(&|r| r.gastro && !r.keyword && !r.deferred)
    );
    let _ = writeln!(
        s,
        "- Removed after picking (D-33): {} duplicate sites (same strand, at most 150 m, larger detourKm), {} within 150 m of a Rastanlage centroid",
        c.site_duplicates, c.at_rastanlage
    );
    let nobrand: BTreeMap<_, _> = x
        .placed
        .iter()
        .filter(|p| p.brands.is_empty())
        .map(|p| x.cands[p.cand].obj)
        .map(|o| (o, ()))
        .collect();
    let _ = writeln!(s, "- Autohöfe without brands: {}", nobrand.len());
    let _ = writeln!(
        s,
        "- Amenities (cluster picks): shower {}, fastCharger {}, open24h {}; mean distance to the junction {:.0} m",
        x.placed.iter().filter(|p| p.shower).count(),
        x.placed.iter().filter(|p| p.fast_charger).count(),
        x.placed.iter().filter(|p| p.open24h).count(),
        x.placed.iter().map(|p| p.dist_m).sum::<f64>() / x.placed.len().max(1) as f64
    );
    let mut det: Vec<f64> = x.entries.iter().map(|e| e.detour_km).collect();
    det.sort_by(f64::total_cmp);
    if !det.is_empty() {
        let _ = writeln!(
            s,
            "- detourKm: min {:.1}, median {:.1}, max {:.1}",
            det[0],
            det[det.len() / 2],
            det[det.len() - 1]
        );
    }

    let _ = writeln!(
        s,
        "\n### Additional Autohof candidates\n\nQualified candidates that did not become an Autohof of the cluster: {}\n",
        x.additional.len()
    );
    if !x.additional.is_empty() {
        let _ = writeln!(
            s,
            "| Name | OSM | Cluster | Distance (m) | Why |\n|---|---|---|---:|---|"
        );
        for ad in x.additional.iter().take(MAX_ROWS) {
            let cd = &x.cands[ad.cand];
            let _ = writeln!(
                s,
                "| {} | {} | {} | {:.0} | {} |",
                cd.name.as_deref().map_or("(no name)".to_string(), cell),
                link(&cd.obj),
                cell(&cluster_label(&x.clusters[ad.cluster])),
                ad.dist_m,
                cell(&ad.why)
            );
        }
        if x.additional.len() > MAX_ROWS {
            let _ = writeln!(s, "\n(showing {} of {})", MAX_ROWS, x.additional.len());
        }
    }

    let _ = writeln!(
        s,
        "\n### Removed Autohöfe (site dedupe, Rastanlage)\n\nAutohöfe dropped after picking: {}\n",
        x.dropped.len()
    );
    if !x.dropped.is_empty() {
        let _ = writeln!(s, "| Name | OSM | Cluster | Why |\n|---|---|---|---|");
        for dr in x.dropped.iter().take(MAX_ROWS) {
            let cd = &x.cands[dr.cand];
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} |",
                cd.name.as_deref().map_or("(no name)".to_string(), cell),
                link(&cd.obj),
                cell(&cluster_label(&x.clusters[dr.cluster])),
                cell(&dr.why)
            );
        }
    }

    let _ = writeln!(
        s,
        "\n### Unplaced Autohof-like services\n\nDeferred services objects farther than 2 km from every junction node: {}\n",
        x.unplaced.len()
    );
    if !x.unplaced.is_empty() {
        let _ = writeln!(s, "| Name | OSM |\n|---|---|");
        for &ui in x.unplaced.iter().take(MAX_ROWS) {
            let cd = &x.cands[ui];
            let _ = writeln!(
                s,
                "| {} | {} |",
                cd.name.as_deref().map_or("(no name)".to_string(), cell),
                link(&cd.obj)
            );
        }
        if x.unplaced.len() > MAX_ROWS {
            let _ = writeln!(s, "\n(showing {} of {})", MAX_ROWS, x.unplaced.len());
        }
    }

    let n = 10.min(x.entries.len());
    let _ = writeln!(
        s,
        "\n### Autohof samples\n\n{} random Autohof entries (seed {}, SplitMix64, partial Fisher-Yates over the Autohof entries in dataset order).\n",
        n, SAMPLE_SEED
    );
    let mut rng = SplitMix64::new(SAMPLE_SEED);
    for (k, idx) in rng
        .sample_indices(x.entries.len(), 10)
        .into_iter()
        .enumerate()
    {
        let e = &x.entries[idx];
        let p = &x.placed[e.placed];
        let cd = &x.cands[p.cand];
        let fuel = cd.fuel.as_ref().map_or("-".to_string(), |(o, _)| link(o));
        let _ = writeln!(
            s,
            "{}. **{}** (autohof), exit `{}`, `{}` km {:.3}, detourKm {:.1}, {}, {:.5}, {:.5}",
            k + 1,
            e.name.as_deref().map_or("(no name)".to_string(), cell),
            cell(e.exit.as_deref().unwrap_or("-")),
            e.strand_id,
            e.km,
            e.detour_km,
            link(&cd.obj),
            e.lat,
            e.lon
        );
        let _ = writeln!(
            s,
            "   - brands: {}; shower: {}; fastCharger: {}; open24h: {}; qualified by: {}; fuel station: {}; POIs within 150 m: {}; cluster {}",
            if e.brands.is_empty() {
                "-".to_string()
            } else {
                e.brands.join(", ")
            },
            yn(e.shower),
            yn(e.fast_charger),
            yn(e.open24h),
            p.reasons.label(),
            fuel,
            p.pois.len(),
            cell(&cluster_label(&x.clusters[p.cluster]))
        );
    }
}

pub fn render(r: &ReportInput<'_>) -> String {
    let mut s = String::new();
    let a = r.assignment;
    let strand_id = |i: usize| r.strands[i].id.as_str();
    let _ = writeln!(s, "# rastpipe build report\n");

    // ---- 1. Summary ----
    let _ = writeln!(s, "## Summary\n");
    let _ = writeln!(s, "- Inputs: {}", r.inputs.join(", "));
    let _ = writeln!(
        s,
        "- dataVersion: {} ({})",
        r.data_version, r.data_version_source
    );
    let _ = writeln!(s, "- generatedAt: {}", r.generated_at);
    let total_km: f64 = r.strands.iter().map(|x| x.length_km).sum();
    let _ = writeln!(
        s,
        "- Strands: {} (total length {:.1} km)",
        r.strands.len(),
        total_km
    );
    let _ = writeln!(s, "- Junctions: {}", r.junction_count);
    let _ = writeln!(
        s,
        "- Facilities by kind: rastanlage {} entries ({} distinct facilities, {} on shared sections), autohof {}",
        r.entries.len(),
        a.placed.len(),
        a.placed.iter().filter(|p| !p.extra.is_empty()).count(),
        r.autohof.map_or("0 (skipped with --no-autohoefe)".to_string(), |x| format!(
            "{} entries ({} distinct)",
            x.counts.entries, x.counts.distinct
        ))
    );
    let _ = writeln!(
        s,
        "- Facility objects found: {} (assigned {}, unassigned {}, Autohof-like deferred {}, duplicates merged away {}, incomplete geometry skipped {})",
        r.total_objects,
        a.placed.len(),
        a.unassigned.len(),
        r.deferred.len(),
        r.merged.iter().map(|g| g.dropped.len()).sum::<usize>(),
        r.skipped_sources
    );
    let _ = writeln!(s, "- POIs considered: {}", r.pois.len());
    let _ = writeln!(
        s,
        "- Amenities (distinct facilities): shower {}, fastCharger {}, open24h {}, no brand {}",
        a.placed.iter().filter(|p| p.shower).count(),
        a.placed.iter().filter(|p| p.fast_charger).count(),
        a.placed.iter().filter(|p| p.open24h).count(),
        a.placed.iter().filter(|p| p.brands.is_empty()).count()
    );
    let _ = writeln!(
        s,
        "- Duplicate objects skipped across inputs: {}",
        r.duplicates
    );

    let _ = writeln!(s, "\n### Brand frequency (distinct facilities)\n");
    let mut freq: BTreeMap<usize, usize> = BTreeMap::new();
    for p in &a.placed {
        for b in &p.brands {
            *freq.entry(*b).or_default() += 1;
        }
    }
    let mut freq: Vec<(usize, usize)> = freq.into_iter().collect();
    freq.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    let _ = writeln!(s, "| Brand | ID | Facilities |\n|---|---|---:|");
    for (b, n) in &freq {
        let _ = writeln!(
            s,
            "| {} | {} | {} |",
            cell(&r.brands[*b].display_name),
            r.brands[*b].id,
            n
        );
    }

    let _ = writeln!(s, "\n### Strands per ref\n");
    let _ = writeln!(s, "| Ref | Strands | Total length (km) |\n|---|---:|---:|");
    let mut per_ref: BTreeMap<u32, (usize, f64)> = BTreeMap::new();
    for st in r.strands {
        let n: u32 = st.ref_.trim_start_matches("A ").parse().unwrap_or(0);
        let e = per_ref.entry(n).or_default();
        e.0 += 1;
        e.1 += st.length_km;
    }
    for (n, (count, len)) in &per_ref {
        let _ = writeln!(s, "| A {n} | {count} | {len:.3} |");
    }
    let _ = writeln!(s, "\n### Strands\n");
    let _ = writeln!(
        s,
        "| ID | Length (km) | Points | First way |\n|---|---:|---:|---:|"
    );
    for st in r.strands {
        let _ = writeln!(
            s,
            "| {} | {:.3} | {} | {} |",
            st.id,
            st.length_km,
            st.points.len(),
            st.first_way_id
        );
    }
    let _ = writeln!(s, "\n### Collected objects\n");
    for (name, n) in &r.collected {
        let _ = writeln!(s, "- {name}: {n}");
    }

    // ---- 2. Invariant violations ----
    let _ = writeln!(s, "\n## Invariant violations\n");
    let _ = writeln!(s, "Count: **{}**\n", r.violations.len());
    if r.violations.is_empty() {
        let _ = writeln!(
            s,
            "None. (Independent assertion over all {} assigned POIs: right of the strand, nearest strand not the opposite direction.)",
            a.placed.iter().map(|p| p.pois.len()).sum::<usize>()
        );
    } else {
        let _ = writeln!(s, "| Facility | POI | Reason |\n|---|---|---|");
        for v in r.violations {
            let _ = writeln!(s, "| {} | {} | {} |", v.facility, v.poi, cell(&v.reason));
        }
    }

    // ---- 3. Unassigned ----
    let _ = writeln!(s, "\n## Unassigned facilities\n");
    let (near, far): (Vec<_>, Vec<_>) = a
        .unassigned
        .iter()
        .partition(|u| u.nearest.is_some_and(|(_, dist, _)| dist <= 300.0));
    let _ = writeln!(
        s,
        "Total: {} (within 300 m of a strand but not on its right: {}; no strand within 300 m: {})\n",
        a.unassigned.len(),
        near.len(),
        far.len()
    );
    if !near.is_empty() {
        let _ = writeln!(s, "### Within 300 m, only on the left\n");
        let _ = writeln!(
            s,
            "| Name | OSM | Nearest strand | Distance (m) | d (m) |\n|---|---|---|---:|---:|"
        );
        for u in near.iter().take(MAX_ROWS) {
            let src = &r.sources[u.src];
            let (si, dist, d) = u.nearest.expect("partitioned");
            let _ = writeln!(
                s,
                "| {} | {} | {} | {:.0} | {:.0} |",
                name_of(&src.tags),
                link(&src.obj),
                strand_id(si),
                dist,
                d
            );
        }
    }
    if !far.is_empty() {
        let _ = writeln!(s, "\n### No strand within 300 m\n");
        let _ = writeln!(
            s,
            "(not on a motorway of the inputs: parking or truck stops at other roads; showing {} of {})\n",
            far.len().min(MAX_ROWS),
            far.len()
        );
        let _ = writeln!(
            s,
            "| Name | OSM | Nearest strand | Distance (m) |\n|---|---|---|---:|"
        );
        for u in far.iter().take(MAX_ROWS) {
            let src = &r.sources[u.src];
            match u.nearest {
                Some((si, dist, _)) => {
                    let _ = writeln!(
                        s,
                        "| {} | {} | {} | {:.0} |",
                        name_of(&src.tags),
                        link(&src.obj),
                        strand_id(si),
                        dist
                    );
                }
                None => {
                    let _ = writeln!(
                        s,
                        "| {} | {} | none | - |",
                        name_of(&src.tags),
                        link(&src.obj)
                    );
                }
            }
        }
    }

    // ---- 3a. Autohof-like ----
    let _ = writeln!(
        s,
        "\n## Autohof-like services (deferred to phase 4)\n\nCount: {} (name/operator/brand contains Autohof, Truck or Maxi, or |d| > 150 m to the chosen strand; input for the Autohof logic)\n",
        r.deferred.len()
    );
    if !r.deferred.is_empty() {
        let _ = writeln!(
            s,
            "| Name | OSM | Strand | d (m) | Reason |\n|---|---|---|---:|---|"
        );
        for d in r.deferred.iter().take(MAX_ROWS) {
            let (st, dd) = match d.strand {
                Some((si, dv)) => (strand_id(si).to_string(), format!("{dv:.0}")),
                None => ("-".to_string(), "-".to_string()),
            };
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} |",
                cell(&d.name),
                link(&d.obj),
                st,
                dd,
                cell(&d.reason)
            );
        }
        if r.deferred.len() > MAX_ROWS {
            let _ = writeln!(s, "\n(showing {} of {})", MAX_ROWS, r.deferred.len());
        }
    }

    // ---- 3b. Merged duplicates ----
    let _ = writeln!(
        s,
        "\n## Merged duplicates\n\nGroups: {}, merged-away objects: {}\n",
        r.merged.len(),
        r.merged.iter().map(|g| g.dropped.len()).sum::<usize>()
    );
    if !r.merged.is_empty() {
        let _ = writeln!(
            s,
            "| Kept | Name | Strand @ km | Merged away |\n|---|---|---|---|"
        );
        for g in r.merged.iter().take(MAX_ROWS) {
            let list: Vec<String> = g
                .dropped
                .iter()
                .map(|(o, n)| format!("{} ({})", link(o), cell(n)))
                .collect();
            let _ = writeln!(
                s,
                "| {} | {} | {} @ {:.3} | {} |",
                link(&g.kept),
                cell(&g.name),
                strand_id(g.strand),
                g.km,
                list.join(", ")
            );
        }
    }

    // ---- 4. Both sides ----
    let _ = writeln!(s, "\n## Facilities on both sides\n");
    let _ = writeln!(s, "Count: {}\n", a.both_sides.len());
    if !a.both_sides.is_empty() {
        let _ = writeln!(
            s,
            "| Name | OSM | Assigned to | Rule | Own and opposite strand (d in m) |\n|---|---|---|---|---|"
        );
        for b in a.both_sides.iter().take(MAX_ROWS) {
            let src = &r.sources[b.src];
            let list: Vec<String> = b
                .strands
                .iter()
                .map(|(si, d)| format!("{} ({:.0})", strand_id(*si), d))
                .collect();
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} |",
                name_of(&src.tags),
                link(&src.obj),
                strand_id(b.assigned),
                b.reason,
                list.join(", ")
            );
        }
    }

    // ---- 5. Shared sections ----
    let shared: Vec<_> = a.placed.iter().filter(|p| !p.extra.is_empty()).collect();
    let _ = writeln!(s, "\n## Shared sections\n");
    let _ = writeln!(s, "Count: {}\n", shared.len());
    if !shared.is_empty() {
        let _ = writeln!(s, "| Name | OSM | Entries (strand, km) |\n|---|---|---|");
        for p in shared.iter().take(MAX_ROWS) {
            let src = &r.sources[p.src];
            let mut list = vec![format!("{} @ {:.3}", strand_id(p.strand), p.proj.km)];
            list.extend(
                p.extra
                    .iter()
                    .map(|(si, km)| format!("{} @ {:.3}", strand_id(*si), km)),
            );
            let _ = writeln!(
                s,
                "| {} | {} | {} |",
                name_of(&src.tags),
                link(&src.obj),
                list.join(", ")
            );
        }
    }

    // ---- 6. Excluded by invariant ----
    let _ = writeln!(s, "\n## POIs excluded by invariant\n");
    let _ = writeln!(
        s,
        "Count: {} (candidates of a corridor or polygon that were filtered out; these are not violations)\n",
        a.excluded.len()
    );
    if !a.excluded.is_empty() {
        let _ = writeln!(
            s,
            "| Facility | POI | POI name/brand | d (m) | Reason |\n|---|---|---|---:|---|"
        );
        for e in a.excluded.iter().take(MAX_ROWS) {
            let poi = &r.pois[e.poi];
            let label = poi
                .tags
                .get("name")
                .or_else(|| poi.tags.get("brand"))
                .unwrap_or("-");
            let _ = writeln!(
                s,
                "| {} | {} | {} | {:.0} | {} |",
                link(&r.sources[e.src].obj),
                link(&poi.obj),
                cell(label),
                e.d,
                e.reason
            );
        }
        if a.excluded.len() > MAX_ROWS {
            let _ = writeln!(s, "\n(showing {} of {})", MAX_ROWS, a.excluded.len());
        }
    }
    let _ = writeln!(s, "\n### POIs claimed by several facilities\n");
    let _ = writeln!(
        s,
        "Count: {} (assigned to the facility with the smallest |d_poi - d_centroid|)\n",
        a.multi_claims.len()
    );
    for m in a.multi_claims.iter().take(MAX_ROWS) {
        let others: Vec<String> = m.others.iter().map(|o| link(&r.sources[*o].obj)).collect();
        let _ = writeln!(
            s,
            "- {} -> {} (also claimed by {})",
            link(&r.pois[m.poi].obj),
            link(&r.sources[m.winner].obj),
            others.join(", ")
        );
    }

    // ---- 7. Unmatched brands ----
    let _ = writeln!(s, "\n## Unmatched brands\n");
    let mut um: Vec<(&String, &(usize, std::collections::BTreeSet<usize>))> =
        a.unmatched_brands.iter().collect();
    um.sort_by(|x, y| y.1 .0.cmp(&x.1 .0).then(x.0.cmp(y.0)));
    let _ = writeln!(
        s,
        "`brand` values of facilities and assigned POIs that match no entry of brands.json: {} distinct values.\n",
        um.len()
    );
    let _ = writeln!(
        s,
        "| brand value | Occurrences | Distinct facilities |\n|---|---:|---:|"
    );
    for (v, (n, facs)) in um.iter().take(MAX_ROWS) {
        let _ = writeln!(s, "| {} | {} | {} |", cell(v), n, facs.len());
    }

    // ---- 8. Remnants ----
    let _ = writeln!(s, "\n## Discarded remnants\n");
    let _ = writeln!(s, "Chains of 2 km or less: {}\n", r.remnants.len());
    if !r.remnants.is_empty() {
        let _ = writeln!(s, "| Ref | Length (km) | Way IDs |\n|---|---:|---|");
        for rem in r.remnants {
            let ways: Vec<String> = rem.way_ids.iter().map(|w| w.to_string()).collect();
            let _ = writeln!(
                s,
                "| {} | {:.3} | {} |",
                rem.ref_,
                rem.length_km,
                ways.join(", ")
            );
        }
    }

    // ---- 9. Autohoefe ----
    autohof_section(&mut s, r);

    // ---- 10. Memory and runtime ----
    let _ = writeln!(s, "\n## Memory and runtime\n");
    let _ = writeln!(
        s,
        "| Pass | Time (s) | Peak RSS after pass |\n|---|---:|---:|"
    );
    for p in r.passes {
        let rss = p.peak_rss_kb.map_or("n/a".to_string(), mb);
        let _ = writeln!(s, "| {} | {:.2} | {} |", p.name, p.seconds, rss);
    }
    let _ = writeln!(s, "\n- Total runtime: {:.2} s", r.total_seconds);
    let _ = writeln!(
        s,
        "- Peak RSS (VmHWM): {}",
        r.peak_rss_kb.map_or("n/a".to_string(), mb)
    );

    // ---- 11. Samples ----
    let _ = writeln!(
        s,
        "\n## Samples\n\n{} random facility entries (seed {}, SplitMix64, partial Fisher-Yates over the entries in dataset order).\n",
        SAMPLE_COUNT.min(r.entries.len()),
        SAMPLE_SEED
    );
    let mut rng = SplitMix64::new(SAMPLE_SEED);
    for (n, idx) in rng
        .sample_indices(r.entries.len(), SAMPLE_COUNT)
        .into_iter()
        .enumerate()
    {
        let e = &r.entries[idx];
        let p = &a.placed[e.placed];
        let src = &r.sources[p.src];
        let _ = writeln!(
            s,
            "{}. **{}** (rastanlage), `{}` km {:.3}, [{}]({}), {:.5}, {:.5}",
            n + 1,
            e.name.as_deref().map_or("(no name)".to_string(), cell),
            e.strand_id,
            e.km,
            e.id.split('@').next().unwrap_or(&e.id),
            src.obj.url(),
            e.lat,
            e.lon
        );
        let _ = writeln!(
            s,
            "   - brands: {}; shower: {}; fastCharger: {}; open24h: {}",
            if e.brands.is_empty() {
                "-".to_string()
            } else {
                e.brands.join(", ")
            },
            yn(e.shower),
            yn(e.fast_charger),
            yn(e.open24h)
        );
        let pois: Vec<String> = p
            .pois
            .iter()
            .take(12)
            .map(|pi| {
                let poi = &r.pois[*pi];
                let label = poi
                    .tags
                    .get("name")
                    .or_else(|| poi.tags.get("brand"))
                    .unwrap_or("-");
                format!(
                    "{} {} ({})",
                    link(&poi.obj),
                    cell(label),
                    poi.tags.get("amenity").unwrap_or("-")
                )
            })
            .collect();
        let _ = writeln!(
            s,
            "   - assigned POIs ({}): {}",
            p.pois.len(),
            if pois.is_empty() {
                "-".to_string()
            } else {
                pois.join("; ")
            }
        );
    }
    s
}
