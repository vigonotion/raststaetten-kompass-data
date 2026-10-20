# Phase 2 data review (regional dataset, dataVersion 2026-10-18)

Reviewer: data-reviewer. Scope: `pipeline/out/regional/dataset.json` + `report.md` (Hamburg + Niedersachsen + NRW), pipeline source in `pipeline/src/`, targets `docs/heuristics.md` §3–6/§9, `docs/schema.md`, `docs/spec.md` §5.1–5.6.
No code or data was modified. No internet. OSM links in `report.md` are for the owner's manual check.

## Method (so the numbers can be reproduced)

- `jq` + `python3` (stdlib only) on `dataset.json` (polyline decoding, own projection/sign check).
- For per-object evidence (polygon vertices, tags, assigned POIs, multi-claims, motorway ways) I built a **throw-away copy** of `pipeline/` in the session scratchpad with an extra `dump` sub-command that re-runs the unmodified pipeline code (`read_all`, `strands::build`, `facilities::assign`) on the three PBFs and writes JSONL. The repository was not touched. It reproduces the report exactly: 235 facility objects, 176 placed, 59 unassigned, 44 multi-claims, 1 excluded POI.
- Own re-implementation of the invariant check in Python (section 7) and of the geometry statistics.
- `cargo test` in `pipeline/`: 69 passed, 0 failed. `cargo clippy --all-targets -- -D warnings`: clean. `rastpipe validate --dataset out/regional/dataset.json --brands ../data/brands.json`: OK (152 strands, 2240 junctions, 176 facilities).

## Summary of findings

| # | Sev. | Finding | Short evidence |
|---|---|---|---|
| F1 | **blocker** (before phase 4 / release) | Autohöfe are emitted as `kind: rastanlage` | 36 of 176 entries (20 %) are Autohof-like; of the 30 entries with d ≥ 150 m, 22 are Autohof-named, 4 more are unnamed Autohof twins |
| F2 | **major** | Node + way duplicates of the same site, POIs/brands/amenities split between them | 12 pairs (10 sites) on the same strand within 0.35 km, 12/12 true duplicates; explains 44/44 multi-claimed POIs |
| F3 | **major** | "Facilities on both sides" is meaningless: 116 of 176 | Carriageways are only 2–28 m apart (median 15 m); rule is equivalent to "d_own ≤ ~85 m". Real both-sides cases: 1 |
| F4 | **major** (spec change needed) | Charging brands/fast charger lost: `operator` is never consulted; Ionity without socket tags → `fastCharger=false` | 148 of 243 assigned chargers have no `brand` tag; ~86 facilities would gain a charging brand; 5 facilities have IONITY but `fastCharger=false` |
| F5 | minor→major | Strand splitting: causes are (a) Elbtunnel tubes, (b) ways in interchanges carrying only the crossing road's ref (gaps 13–740 m), (c) real network gaps, (d) foreign (Dutch) `A 7` stub | 152 strands; refs with >1 strand per direction listed in §4 |
| F6 | minor | Unnamed town fuel stations / Autohöfe tagged `highway=services` pass as Rastanlage (d 240 m) | way/147134992 |
| F7 | minor | open24h under-reported (spec-conform): 71 facilities have a 24/7 fuel POI but `open24h=false`; `Mo-Su 00:00-24:00` variants ignored | 19+ POIs |
| F8 | minor | Unmatched-brand statistics count POI occurrences, not facilities; noise (banks, parcel lockers) pollutes the list; "Circle K 12" is one Autohof | §6 |
| F9 | minor | Brand-category mismatches via name fallback (8 cases) | §6 |
| F10 | info | Report/pipeline items: `rastanlage`-only dataset, km clamping at strand ends (2 facilities), 4 facilities with 0 POIs, 7 without brand | §8 |

Everything required by the acceptance numbers holds: **0 invariant violations** (confirmed independently, §7), km within `[0, lengthKm]` for all 176 facilities and all junctions, IDs unique, sign convention of `d` correct (§7).

---

## F1 — Autohöfe emitted as `rastanlage` (blocker before phase 4 / release)

Heuristics §7/spec §5.5 define Autohöfe as `amenity=fuel` candidates that are **not** assigned to a Rastanlage and carry `kind: autohof`. The current assignment (`facilities.rs:446-`, `collect_sources` takes every closed `highway=services` way/relation/node) turns every Autohof that is mapped as `highway=services` into a `rastanlage`.

Evidence (`b4/b5` analysis, name/operator/brand matching `autohof|truck|maxi`, plus unnamed twins):

- **36 of 176** placed entries are Autohof-like (35 by name/operator, 1 via POI name, e.g. `osm-way-99351284`, twin of Autohof Westerlinde). 34 of the 59 unassigned objects are Autohof-named as well (they are just >300 m away).
- d distribution of own-strand distance (Autohof / other): 0–50 m 3/46, 50–100 m 8/75, 100–150 m 3/11, **150–200 m 9/4, 200–250 m 6/4, 250–300 m 7/0**. Of the 8 entries with d ≥ 150 m that are not Autohof-named, 4 are unnamed duplicate ways of an Autohof (28473085 Wendhausen, 94520791 Rhüden, 349215236 Lippetal, 1204979657 Sittensen) and the other 4 are truck/fuel sites (147134992 plain Aral station at A39 km 30.8, 198625855 `Tankpark Moorfleet`, 961635296 `Rosis Leintal`, 323110477 `Oderwald` d 200 m, not verified). No entry that looks like a classic Raststätte (Tank & Rast / Serways / named "…Ost/West/Nord/Süd") is farther than 150 m from its strand.
- Examples with d: Autohof A28 Apen `osm-way-93843430` 265 m, MAXI Autohof Wilnsdorf `osm-way-646510056` 164 m, Autohof Ladbergen `osm-way-218280479` 165 m, Hoyer Autohof Cloppenburger Land `osm-way-1083502417` 292 m.
- Samples 1, 3, 5, 7, 16 of `report.md` are all Autohöfe. The `rastanlage` count in the report summary is therefore inflated by ~20 %; "Autohof 0 (phase 4)" would later double them (same site as `rastanlage` and `autohof`).
- The 300 m candidate radius (`params.rs:22`) is hit by 7 Autohöfe at 250–301 m; 34 Autohöfe lie further out (`Autohof Moorburg` 503 m, `Maxi-Autohof Lauenau` 469 m …). So with the current rule Autohof coverage is arbitrary (half in, half out).

Suggestion (needs orchestrator/owner decision, contracts are read-only for me):
1. In phase 4 classify first: a `highway=services` object whose `name|operator|brand` contains `Autohof` (case-insensitive) **or** whose centroid is d > 150 m from the strand (data: genuine Raststätten ≤ 150 m) becomes an Autohof candidate and is excluded from the Rastanlage pass. Proposed change in `docs/heuristics.md` §4.2: "Objects matching the Autohof name rule (§7) are not Rastanlagen." with the 150 m figure as evidence only (D-xx).
2. Until phase 4 exists: add the 36 to a report section "Autohof-like objects in Rastanlagen (to be handled in phase 4)" so the count is visible, or keep them but tag them (not recommended — consumers see them as Rastanlage).
3. Duplicates (F2) must be collapsed before/while doing this, otherwise the Autohof gets two entries per direction.

## F2 — Duplicate node + way for the same site (major)

Method: all placed facility pairs on the same strand with |Δkm| ≤ 0.35 km, classified by geometry (node inside polygon?) and name.

- **12 pairs, 10 sites, all 12 true duplicates; 0 genuinely different neighbouring sites.** There are no pairs at all between 0.35 and 1.5 km, so the 0.35 km threshold separates cleanly.
- Sites (km on strand, centroid distance): Soester Börde Süd `A44-O-0` 33.94/34.03 (node 632560668 / way 162532158, 90 m), Soester Börde Nord `A44-W-0` 62.10/62.16 (node 2282402778 / way 224226652, 59 m), Nievenheim Ost `A57-NW-0` 21.00/21.15 (node 1797677548 / way 58543624, 146 m), Autohof Sittensen `A1-SW-0` 62.58/62.61/62.69 (node 3542183071 / way 347995377 / unnamed way 1204979657 → 3 entries, 3 pairs), Wendhausen `A2-W-0` 35.32 (node 3560180176 / way 28473085), Euro Rastpark Lippetal `A2-W-0` 261.25 (way 349215236 / node 3551312377), Westerlinde `A39-SW-0` 62.2 (node 3590608193 / way 99351284), Schwarmstedt `A7-S-0` 127.72 (node 3620418627 / way 356620997), Rhüden `A7-S-0` 224.11 (node 3622838853 / way 94520791), Hoyer Autohof Hamburg Süd `A1-NO-0` 447.51 (node 3624017502 / way 432010425).
- In every pair the node lies inside the polygon (except the 3rd Sittensen object, which is 107/121 m away and has its own Shell POI).
- **All 44 multi-claimed POIs belong to these sites** (11 distinct claimant sets: 10 sites + the Sittensen triple). `report.md` presents this as a rare POI-level event, but it is purely the duplicate effect.
- Consequence for content: the POI split follows "smallest |d_poi − d_centroid|" (`facilities.rs:550-575`), i.e. arbitrary between node and polygon. Examples: Westerlinde node `osm-node-3590608193` has 0 POIs and **no brands** while its twin gets Shell + Tesla + café; Rhüden node keeps Esso + MAXI restaurant but its three Tesla chargers went to the way twin → node `fastCharger=false`, way `fastCharger=true` (sample 5); Schwarmstedt node (sample 7) `fastCharger=false` though Aral pulse is on the same site; Soester Börde Süd node: 2 brands, way: 1 brand.
- Node corridor (±150 m, d ∈ [d−30, d+50]) is narrower than the polygon of the same site, which is why the node variants lose POIs systematically.

Suggestion: merge before POI assignment. Rule that fits all 12 pairs: same strand, |Δkm| ≤ 0.35 km (or centroid distance ≤ 150 m) and one object's centroid inside the other's polygon (or both within 150 m and names equal after normalization / one name null). Keep the polygon (or the object with a name) as the primary, take `name` from whichever has one, OR-merge the tags `brand`, `shower`, `opening_hours`. Add a report section "Merged duplicates" (this would also make "POIs claimed by several facilities" empty). Expected effect: 176 → 166 entries.

## F3 — "Facilities on both sides" (116 of 176) is not informative (major)

Orchestrator's suspicion is confirmed and quantified. Stats over the 176 placed facilities (centroid vs own strand and nearest opposite strand of the same ref; python re-implementation, sign as schema §3):

| metric (m) | min | p10 | p25 | p50 | p75 | p90 | p99 | max |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| d_own (= distance, always > 0) | 19.0 | 38.6 | 46.6 | 63.8 | 103.5 | 200.0 | 290.3 | 292.5 |
| distance to opposite strand | 41.0 | 52.9 | 62.2 | 78.3 | 120.3 | 215.2 | 307.4 | 308.0 |
| carriageway separation (dist_opp − dist_own) | 2.1 | 10.7 | 12.6 | 14.7 | 17.1 | 19.1 | 23.2 | 27.5 |

- Every facility has an opposite strand within 400 m; for **all 176** `d_opp` is negative (facility is *left* of the opposite strand) and the centroid is never nearer to the opposite strand than to its own. So the right-hand rule is never in doubt.
- The rule (`facilities.rs:409-441`, `params.rs:23`) fires iff `dist_own ≤ 100 ∧ dist_opp ≤ 100`; because dist_opp ≈ dist_own + 15 m this is exactly "d_own ≤ ~85 m" (max d_own among flagged 86.9 m; min d_own among not flagged 82.4 m). 109 of the 140 non-Autohof facilities are flagged (78 %), 7 of the 36 Autohöfe.
- Polygon geometry (166 polygon facilities, 10 nodes): **no polygon touches or crosses the opposite carriageway polyline**; 16 polygons cross their *own* strand's polyline, but only by 0–8 m (inside the 15 m Douglas-Peucker tolerance; e.g. Remscheid West −8 m, Hohe Mark West −4 m); vertices with d_own < −15 m: **1**.
- The single real both-sides object is `osm-relation-5187212` Autohof Altenwerder (Hamburg harbour): a multipolygon with **two rings on opposite sides of A 7** (ring 1: d −179…−48 m, ring 2: d +21…+200 m), centroid 19 m right of `A7-S-0` km 18.55. Its corridor is d ∈ [−179, 200]; the invariant filter (`d ≤ 0`) drops everything on the left half, so brands/amenities only come from the right half. It is assigned by the right-hand rule as specified.

Proposal (needs a change of `docs/heuristics.md` §4.3 and `docs/spec.md` §5.2.3, hence orchestrator decision): replace "centroid within 100 m of both carriageways" by a geometric test that actually detects bridge/transit facilities:

> report "both sides" iff (a) the polygon outline intersects or contains a segment of the opposite-direction strand of the same ref, **or** (b) at least one polygon vertex has `d_own < −15 m` (15 m = DP tolerance, so snapping noise is excluded), **or** (c) the centroid is nearer to the opposite strand than to its own (not the case for any facility today).

On this dataset (a) → 0, (b) → 1 (Altenwerder), (c) → 0, i.e. 1 entry instead of 116. For nodes use only (c). If the owner prefers to keep a distance criterion, it must be relative to the carriageway separation (`dist_opp − dist_own ≤ 30 m` and `d_own ≤ separation + 30 m`), not absolute 100 m. Note that d_own of a normal Raststätte is 40–85 m, so any absolute threshold ≤ 100 m flags all of them.

## F4 — Brand and fast-charger quality (major)

1. **`operator` is never used for brands.** 148 of 243 assigned `charging_station` POIs have no `brand` tag (only `operator`), 8 have neither. Top operators without brand: `E.ON Drive Infrastructure GmbH` 40, `Ionity` 18 (+2 `ionity`), `EnBW mobility+ AG und Co.KG` 17, `EnBW Energie Baden-Württemberg AG` 13, `EnBW` 8, `Allego` 6, `Shell Deutschland GmbH` 5, `EWE Go(/GmbH)` 5. With an operator fallback restricted to `amenity=charging_station` and the existing `match.brand` strings, an upper-bound estimate (loose regex) gives **92 new (facility, brand) pairs on 86 facilities** (eon-drive 47, enbw 24, ionity 7, ewe-go 6, allego 5, shell-recharge 2, aral-pulse 1). Today only 46 facilities show any charging brand although 137 have at least one charger. Examples: Lüneburger Heide West (sample 2) has an EnBW 300 kW station → no `enbw`; Vierwinden-Süd, Rhynern-Süd, Sauerland-West, Frechen Süd (samples 8, 9, 11, 14) all have E.ON Drive chargers → no `eon-drive`.
   Suggestion: heuristics §5: "for `amenity=charging_station` without a matching `brand`, compare `operator` (same equality rule) before `name`." plus aliases in `brands.json` (see §6).
2. **Ionity (and Tesla/Fastned) without socket tags → `fastCharger=false`.** 5 facilities carry `ionity` but `fastCharger=false`: Harz Ost `osm-way-33804997`, Auetal Süd `89684059`, Ohligser Heide Ost `136296047` (sample 6), Autohof Geseke `163688925`, Dammer Berge West `626717821`. 35 of 243 assigned chargers have no `socket:*`/`maxpower` tag at all; 20 facilities with ≥ 1 charger have `fastCharger=false`. By definition IONITY/Tesla Supercharger/Fastned are HPC. Suggestion: the brands `ionity`, `fastned`, `tesla-supercharger` imply `fastCharger=true` (heuristics §6.4 addition). Also: hyphen variants (`E-on Drive`, sample 15) are not normalised to `E.ON Drive`.
3. Fast-charger prevalence: 117/176 (66 %); 110 via `socket:type2_combo ≥ 1`, 7 via power only; 11 chargers have `type2_combo` without any power tag. Values seen (`300 kW` 38, `350 kW` 41, `50 kW` 38, `socket:cgs:output=50` typo key in sample 12) look plausible; 79 of the 117 have no charging brand, which is the operator problem above, not an error in the rule.
4. Spec nuance: `socket:type2_combo` counts any combo plug regardless of power (spec says so). Fine for MVP, noted because 11 chargers rely on it alone.

## F5 — Strands: per ref and direction (explanations for the "suspicious" cases)

Strands per ref (152 total, 7763 km). Refs with more than one strand per **direction family** (direction label = overall bearing, so e.g. `NW`/`N` of the same road may be one family; I merged by chain continuity, gap = end of X to start of Y):

| Ref | Strands (length km) | Gap in chain | Probable cause |
|---|---|---|---|
| A 7 | N-0 294.0 / S-0 293.9 (main) | – | – |
| A 7 | **NW-0 3.44 / SO-0 3.59** | strands start at the same node as the main tube way (`53.53499,9.93233`); geometry within mean 10 m (max 23 m) of N-0, mean 30 m (max 76 m) of S-0 | **Elbtunnel tubes.** OSM has 4 separate tunnel ways (N: 33817648, 219334253; S: 3127133, 33817647, each ≈ 3.3 km, all `highway=motorway`, `ref=A 7`, oneway forward, no `oneway=no`). The longest-continuation rule (heuristics §3.2) takes one tube per direction into the main strand, the other tube stays a 3.4 km chain > 2 km and becomes its own strand. No facility there. |
| A 7 | NO-0 21.9 / SW-0 20.8 | NO→N 36 m; S→SW 615 m | Hessen/Niedersachsen border near Hann. Münden–Lutterberg (51.40N 9.81E): the part south of the border is a separate chain; southbound gap 615 m = ways in the interchange not carrying `ref=A 7`. Facility `Autohof Lutterberg` is on SW-0 km 14.4, so the Niedersachsen/Hessen seam splits the list. |
| A 7 | **O-0 2.02 / W-0 2.03** | – | Probably the Dutch A7 stub at the Bunde/Nieuweschans border (53.18N 7.20E), i.e. a foreign road with the German-looking ref `A 7`; includes facility `Poort van Groningen` (`osm-way-198877489`, A7-W-0 km 0.7). Both are just above the 2.0 km remnant threshold. **Assumption: Dutch**, owner to check on OSM. Suggest dropping chains whose bounding box is > 100 km from the rest of the ref or adding a "foreign ref" report. |
| A 3 | N-0 120.1 + NW-0 65.0; S-0 117.6 + SO-0 65.0 | 572 m (N→NW); 742 m (SO→S) | Oberhausen interchange (A3/A2/A516 area, 51.53N 6.83E): connecting ways carry only the crossing ref (e.g. way 31166165 `[2]` starts on the chain end node), so the A 3 chain ends. Not a bug of chaining, a "ref gap" (heuristics §10). |
| A 4 | O-0 80.1 + O-1 68.6; W-0 83.7 + W-1 68.7; plus NO-0 3.2, NW-0 2.6 | 13–49 m | Köln: A 4 shares ways with A 3 (`[3,4]` 20 ways, `[4,59]` 5) and the chain splits inside the shared stretch; the two short pieces are Köln ramps/loops with `ref=A 4`. |
| A 28 | NW-0 41.7 + O/W 53.5/53.8 | 270–330 m | Oldenburg interchange. |
| A 31 | N-0 209.2 / NW-0 30.7 | 432 m | Leer/Bunde interchange. |
| A 39 | NO/SW 68 + NW/SO 36 | 27–543 m | real network gap (A 39 Lüneburg–Wolfsburg not continuous in the data) plus interchange. |
| A 44 | NO-0 56.3, NO-1 16.1, O-1 37.6, O-0 96.2, NW-0 10.6, … = **10 strands** | 13–310 m | A 44 is genuinely discontinuous (Aachen–Mönchengladbach, Düsseldorf–Velbert, Dortmund–Kassel, Kassel). Plausible. |
| A 45 | NW-0 76.1 + NW-1 47.8, but SO-0 124.0 in one piece | 58 m (NW only) | Directional asymmetry: southbound chain is continuous, northbound breaks at 51.263N 7.603E (way 135005123 ends, next way 1137900568 starts 58 m later, 96 m-remnant way 264018960 sits in between without shared node). |
| A 46 | 8 strands (two chains per direction) | 24–424 m | interchanges + unbuilt sections. |
| A 52 | 6 | 12–43 m | interchanges. |
| A 59 | N-0/-1/-2 and S-0/-1/-2 | 6–184 m | interchange gaps. |
| A 61 | **NW-0/-1/-2 (3), SO-0/-1 (2)** | NW-2→NW-1 67 m | Kreuz Bliesheim (A1×A61): the connector ways are tagged `[1,61]`/`[1]` only (way 152263995 `[1]` starts exactly at the end of NW-2), so the A 61 chain breaks on one carriageway but not on the other. |
| A 565 | N-0 22.1 + NO-0 3.3 | 66 m | normal. |
| A 516 | NW-0 3.0 vs SO-0 5.5 | – | unequal lengths; 3 `oneway=no` ways (also 4 on A 293, 1 on A 4) plus 2 remnants 1.7 + 0.4 km. Worth a look in phase 3. |

Per-direction facility counts (Autohöfe and duplicates included): A 1 NO 21 / SW 18, A 2 O 13 / W 17, A 7 N 9 / **S 15**, A 30 3/3, A 44 O 4 / W 5, A 57 3/2, A 3 N 5, NW 2, S 3, SO 2. The A 7 imbalance is explained by 8 Autohof-like entries on S-0 vs 2 on N-0; the Raststätten mirror well (e.g. A 7 Harburger Berge Ost km 263.4 ↔ West km 30.5: 294.0 − 30.5 = 263.5; same for Lüneburger Heide, Harz, Allertal, Hannover-Wülferode, Hildesheimer Börde, Göttingen).
Raststätten with **no twin** on the opposite strand within 3 km (candidates for missing `highway=services` tagging or the 300 m cut-off; owner check on OSM): `Vorm Wietzenbruch` (A7-S km 89.2, no object at the mirror point km ≈ 204.8 within 4 km except unassigned Autohof Soltau Süd), `Raststätte Lennhof` (A1-SW 350.9), `Buddenkuhle` (A1-SW 254.5, relation), `Raststätte Vellern` (A2-O 99.8), `Autobahnraststätte Beverbach` (A40-O 84.5), `Raststätte Hösel` (A3-N 93.1), `Knauheide` (A3-SO 0.4), `Bottrop` (A2-O 4.2). 30 of 146 facilities with d ≤ 150 m have no twin; ~8 of them are real candidates, the rest Autohöfe/duplicates.

Impact: strand splits only matter for the "list ends at strand end" behaviour (heuristics §10), not for correctness. Elbtunnel twin strands are harmless for the matcher in the hysteresis design (same direction, 10–30 m apart, no facilities) but they do violate the idea "one strand per direction"; suggest documenting it, or skipping chains that are within 30 m of an existing chain of the same ref and direction (they are then reported as "parallel duplicates").

## Check-list items from the assignment

### Invariant stage 2 independence (OK, with caveats)
`invariant.rs` shares no code with `facilities.rs` / `index.rs` / `geo/local.rs`: own `seg_geometry` (`invariant.rs:39-56`), brute-force `nearest_segment` over all strands (`:76-105`), own bearing gap. It uses the same constants (window 2 km, 135°) and the same rule, so it detects implementation errors but not specification weaknesses. It only checks POIs (not the facility's own `d > 0`, which is trivially true: all 176 d_own ∈ [19, 292] m). `main.rs:190-203` feeds it `p.proj.km` from stage 1 as the window centre (shared input, minor).
I re-implemented the check separately in python (own windowed d on the assigned strand, brute-force nearest strand with same-ref + bearing gap > 135°): **1025 assigned POIs, 0 violations** (same count as `report.md`). POI d on own strand: min 16.3 m, p1 21.5 m, p50 73 m, max 423 m; no POI with d < 15 m.
The single stage-1 exclusion `osm-node-339353232` ("Truck Service", Shell, d 153 m) for facility `osm-way-1273344781` (unnamed, `A61-NW-0` km 1.4) is a true positive (nearest strand is the opposite direction). Suggest the report list the facility name/strand.

### Corridor one-sided, node bounds, sign of d (OK)
- Polygon: `Corridor::Polygon.contains` `s ∈ [s_min,s_max]`, `d ∈ [d_min, d_max+50]` (`facilities.rs:348-356`); `d_min` can be ≤ 0 (17 polygons have d_min < 0, 2 below −5 m), but stage 1 discards `d ≤ 0` also inside the polygon (`facilities.rs:519`). No buffer towards the carriageway exists. The +50 m outer buffer is used by only 23 of 993 POIs (970 are inside the polygon); 3 are > 50 m from the nearest polygon vertex.
- Node: `|s−s_node| ≤ 0.150 km`, `d ∈ [max(0, d−30), d+50]` (`facilities.rs:357-359`) — exactly spec §5.3. Used by 10 node-only facilities (+ the node twins from F2).
- Sign: `signed_offset_xy` = `−cross/len` (`geo/local.rs:36-43`) = schema §3. Cross-check with data: Soester Börde Süd (south side) is on `A44-O-0` (eastbound, right = south), Nord on `A44-W-0`; for all 176 facilities `d_own > 0` and `d_opp < 0`; for all 1025 POIs `d > 0` (python re-computation).
- Two soft spots: d is measured against the **simplified** (15 m DP) strand, so d has ±15 m noise at the 0-line (16 polygons "cross" their own carriageway by 0–8 m); and `d` for a point beyond a strand end is the line offset of the clamped segment (see F10).

### km in [0, lengthKm]
0 violations over 176 facilities (`jq`); 0 negative junction km. Two facilities sit on the strand ends because their centroid projects beyond the end (clamped): both `Grenzrastplatz Aire frontalière de Lichtenbusch` (`osm-way-493659200` `A44-NO-0` km 0.000, `osm-way-1010044669` `A44-SW-0` km 55.866 = lengthKm). Neither will ever be "ahead" of a vehicle (`km > currentKm`); harmless, border rest area.

### Facilities without brands (7) and with 0 POIs (4)
No-brand facilities: Autohof Westerlinde node (twin, F2), SVG Autohof Köln-Eifeltor `osm-way-53644421` (brand `Independent`), Raststätte Vellern `osm-way-150477188` (operator Tank & Rast but no `brand`), Eichengrund `osm-way-162532530` (Innogy), ARAL Truckparking `osm-way-183111969` (parking, 0 POIs), Hoyer Autohof Cloppenburger Land (brand `Hoyer`), Buddenkuhle `osm-relation-13293730` (0 POIs). Three of seven are fixed by F2/F4/brands.json; `operator = Tank & Rast / Autobahn Tank & Rast Gruppe` could also map to `tank-und-rast` (brand list has `match.brand` only; 6 facilities today).
Facilities with 0 assigned POIs: 4 (Westerlinde node, ARAL Truckparking `osm-way-183111969`, Buddenkuhle `osm-relation-13293730`, and `Serways Raststätte Rölvedermühle West` `osm-way-271936861`, A45-SO-0 km 43.9, which has only the `serways` brand from its own tag — worth a manual look: a Serways site without any POI points to a too-small polygon or missing POIs).

### Amenity plausibility
- shower 110/176 (62.5 %): 64 from `amenity=shower` only, 23 amenity + facility tag, 8 facility tag only, 13 `shower=yes` on SANIFAIR toilets (and one on fuel). Plausible for motorway service areas.
- open24h 51/176 (29 %): 24 via restaurants, 8 via facility tag, rest mixed. **Conservative by spec:** 71 facilities have a 24/7 *fuel* POI but `open24h=false` (fuel is not gastronomy), and `Mo-Su,PH 00:00-24:00` (10), `Mo-Su 00:00-24:00` (5), `PH,Mo-Su 00:00-24:00` (4) are equivalent to 24/7 but ignored. Suggest (owner decision): normalise those three strings; consider counting `fuel` with 24/7 as well, otherwise the 24h filter hides most Tank & Rast sites at night.
- fastCharger: see F4.

### Samples (20 in `report.md`): trace of strand / brands / amenities
Strand rule everywhere: candidate strands within 300 m, `d > 0`, smallest |d|; the opposite strand has `d < 0` in all 20 cases. d = distance to the assigned strand; "opp" = signed d to the opposite strand.

| # | Facility (`osm-…`) | Strand, km, d (opp) | Why these brands / amenities |
|---|---|---|---|
| 1 | way/93843430 Autohof A28 Apen | A28-W-0 36.016, d 265 (−279) | **Autohof** (F1). aral: fuel brand; aral-pulse: charger brand tag; mcdonalds: way brand; rewe-to-go: node w/o amenity with brand tag. fastCharger: EWEgo `type2_combo=4`, 300 kW; open24h: McDonald's 24/7. EWE Go unmatched (operator/name only) |
| 2 | way/314084533 Lüneburger Heide West | A7-S-0 71.037, d 92 (−111) | mcdonalds, nordsee, shell via brand tag; coffee-fellows brand; ionity brand tag; sanifair toilets; shower: SANIFAIR toilets `shower=yes` (4846738941); fastCharger: Ionity + EnBW `combo 3/4`, 300 kW; open24h: Coffee Fellows 24/7. EnBW charger ("Brunautal West Ladestation", operator only) gives no `enbw` (F4); its name points to another Rastplatz, check |
| 3 | way/646510056 MAXI Autohof Wilnsdorf | A45-SO-0 116.989, d 164 (−171) | **Autohof**. burger-king, totalenergies (brand `TotalEnergies`), tesla (brand `Tesla Supercharger`, `combo=8`, 250 kW). Car wash `Cologne Truck Wash` came in via the +50 m buffer (40 m outside polygon), hotel `Qualitel` via `brand` tag only |
| 4 | way/296611541 Brockbachtal Nord | A30-W-0 76.827, d 57 (−69) | segafredo (cafe), westfalen (way/567779534 brand `Westfalen`); node/3736953903 has `brand=Westpfahlen` (unmatched, almost certainly a typo of Westfalen: owner assumption); sanifair; fastCharger: mblty `combo=6`, 400 kW; shower: `amenity=shower` node; open24h: restaurants 24/7 |
| 5 | node/3622838853 Autohof Rhüden | A7-S-0 224.112, d 166 (−176) | **Autohof**, **duplicate** of way/94520791 (F2): only Esso + MAXI restaurant stayed; the 3 Tesla/other chargers went to the twin → `fastCharger=false`; shower via facility tag; open24h via MAXI restaurant 24/7 |
| 6 | way/136296047 Raststätte Ohligser Heide Ost | A3-N-0 73.148, d 48 (−61) | services way has `brand=Serways`; BK, Coffee Fellows, sanifair via brand; shell from fuel; **aral from a restaurant tagged `brand=Aral`** (F9, likely tagging error); ionity brand, but Ionity node has no socket tags → `fastCharger=false` (F4.2); shower facility tag + `amenity=shower` |
| 7 | node/3620418627 Autohof Schwarmstedt | A7-S-0 127.722, d 254 (−265) | **Autohof**, **duplicate** of way/356620997. aral from way/195968389 (no amenity, brand `Aral`, `shower=yes` → shower true); Aral pulse charger went to the twin → `fastCharger=false` |
| 8 | way/621681780 Vierwinden-Süd | A46-NO-0 39.532, d 56 (−66) | serways from the services way's own brand; shell from two fuel ways; segafredo cafe; fastCharger: E.ON `combo=2`, 160 kW; E.ON brand missing (operator only, F4) |
| 9 | way/114958937 Rhynern-Süd | A2-O-0 75.719, d 64 (−81) | mcdonalds, nordsee, coffee-fellows, esso, sanifair via brand tags; fastCharger: E.ON `combo=2`, 50 kW; shower: facility tag + `amenity=shower` |
| 10 | way/147134992 (no name) | A39-SO-0 30.834, d 240 (−250) | polygon d range 186–280 m: an ordinary Aral station with ATM, car wash and Aral pulse chargers (F6). aral, aral-pulse by brand tags; fastCharger: Aral pulse `combo`, 300 kW |
| 11 | way/380407180 Rastplatz Sauerland-West | A45-SO-0 54.539, d 69 (−77) | esso (brand), sanifair (node w/o amenity, brand `Sanifair`), serways (way, brand `Serways`, no amenity); fastCharger: E.ON `combo=2`, 50 kW |
| 12 | way/216996611 Huntetal West | A29-S-0 70.461, d 59 (−71) | westfalen (brand `WESTFALEN`, case-insensitive); fastCharger: EnBW with `socket:cgs:output=50` (typo key, accepted by the `socket:*:output` rule); shower: `amenity=shower`; open24h restaurant 24/7 |
| 13 | way/709237371 Obergassel | A2-O-0 135.559, d 40 (−59) | serways (fast_food, toilets, shower, way: all `brand=Serways`), coffee-fellows, eon-drive (brand `E.ON Drive`, `combo=2`, 100 kW); shower: facility tag + `amenity=shower` |
| 14 | way/646491880 Frechen Süd | A4-O-0 58.009, d 37 (−53) | mcdonalds, segafredo, totalenergies (brand), sanifair; serways **by name** (`Serways Frechen Süd`, restaurant has no brand); fastCharger: E.ON `combo=2`, 50 kW |
| 15 | way/195132146 Grundbergsee Süd | A1-NO-0 393.121, d 50 (−65) | polygon d_max 201 m (large polygon); coffee-fellows by **name** (no brand tag); charger named `E-on Drive` unmatched (hyphen) and no socket tags → `fastCharger=false` although E.ON charger exists; shower: SANIFAIR `shower=yes` + `amenity=shower`; open24h restaurant/fast_food 24/7 |
| 16 | way/218280479 Autohof Ladbergen | A1-NO-0 230.628, d 165 (−179) | **Autohof**. aral, aral-pulse (brand tags), BK, McD; EWE Go charger 150 kW combo=2; open24h via facility `opening_hours=24/7` |
| 17 | way/109506886 Vorm Wietzenbruch | A7-S-0 89.183, d 87 (−99) | serways by **name** (restaurant `Serways`), coffee-fellows, eon-drive (charger `brand=E.ON`, `combo=1`, 50 kW); no twin on A7-N-0 (F5) |
| 18 | node/1797677548 Serways Rasthof Nievenheim Ost | A57-NW-0 21.003, d 51 (−64) | node facility: corridor `s ± 0.15 km`, `d ∈ [21, 101]`; serways from the node's own brand tag; **duplicate** of way/58543624; shell fuel, sanifair, lavazza, shower node; fastCharger false (no charger inside the narrow node corridor) |
| 19 | way/242506973 Salzgitterhüttenblick Nord | A39-SW-0 46.621, d 82 (−89) | mcdonalds (brand), aral (fuel way brand); `Bistro` fast_food has no brand; no charger |
| 20 | way/654401635 Raststätte Lennhof | A1-SW-0 350.867, d 37 (−52) | coffee-fellows (brand); charger `brand=Innogy` unmatched (F6/§6), `combo=2`, 50 kW → fastCharger true; no fuel POI inside the polygon |

Result of the trace: strand, km and d are as defined in all 20 cases; all brands/amenities can be explained from the assigned POIs. 5 of 20 samples are Autohöfe (F1), 3 involve duplicate twins (F2), 3 show the missing operator/HPC logic (F4).

## 6. Brands

Top unmatched `brand` values (`report.md`, "Frequency" = number of POI occurrences, not facilities; the brand table above it counts distinct facilities, so the two lists are not comparable):

| brand value | occ. | facilities | proposal | notes |
|---|---:|---:|---|---|
| Circle K | 12 | **1** (Break Autohof Hamburg-Nordheide, 12 POI nodes) | add `circle-k`, category `fuel`, `match.brand ["Circle K"]` | needs owner OK for slug (`simpleIconsSlug: null` unless verified). One site, so low priority |
| Innogy | 5 | 5 | alias into `eon-drive` (`match.brand` + `"Innogy"`), operator on these POIs is already E.ON Drive Infrastructure | assumption: Innogy charging was transferred to E.ON |
| e-on, eon, E.ON Drive GmbH, E.ON Drive Infrastructure GmbH | 3+1+1+1 | – | alias into `eon-drive` | normalisation does not unify `e-on` with `E.ON` |
| EWE Go GmbH, EWE2Go, EWEgo | 2+1+1 | – | alias into `ewe-go` | |
| EnBW HyperNetz | 1 | – | alias into `enbw` | |
| Total EV Charge | 1 | – | alias into `totalenergies` or a charging brand; owner decision | |
| Hoyer | 2 | 1 | `hoyer`, fuel (Autohof operator). Optional | Autohof logic (phase 4) may cover it |
| ROSIs / Rosis | 2 | 1 | optional `rosis`, food; Autohof chain in Niedersachsen (assumption) | |
| Westpfahlen | 2 | 2 | **do not add**: both on Brockbachtal Nord/Süd (A30), next to `Westfalen` stations — likely OSM typo; report to the owner for an OSM fix | |
| Independent | 2 | 2 | skip (fuel brand name, no icon) | |
| bft, Premio, Tabilo, Roatel, Vigo, Knauber Bio-LNG, mer/mblyt | 1 each | | skip for MVP | |
| Postbank, ING(-DiBA), Sparkasse, Commerzbank, Volksbank, Amazon Hub, DHL Packstation, Euronet, Cash, cash zone, TollCollect, Deutsche Post, DLRG, DEKRA, Löwen Play | ≈ 30 | | **noise**: POIs only collected because they carry a `brand` tag (atm 15, parcel_locker 6, car_wash 4, vending_machine 3 …) | exclude from the unmatched list: only report unmatched brands of POIs with `amenity ∈ {fast_food, restaurant, cafe, fuel, charging_station}` or without `amenity` but within a brand category |

`tesla-supercharger`: `match.brand` contains the generic `Tesla`; 8 facilities carry it, all 8 Tesla POIs are real `Tesla Supercharger` stations (brand `Tesla Supercharger` / name `… Supercharger`), so there is no false positive in the data. Risk remains for `Tesla` shops/service centres tagged `brand=Tesla` (they have no `amenity` and would still be collected as POI): suggest removing `Tesla` or requiring `amenity=charging_station` for that brand. Same logic for the other bare names in `brands.json` (`star`, `bp`, `hem`, `q1`: acceptable).

Missing brand IDs worth checking in the next full run (from the table): none beyond the above; `marche`, `moevenpick`, `starbucks`, `costa-coffee` etc. do not occur in these three states' assigned POIs.

F9 — brand/category mismatches via name fallback (8 cases): chargers named `Shell Ladestation`/`Aral Ladestation`/`Aral Autohof Geseke` become fuel brands (`shell`, `aral`) instead of `shell-recharge`/`aral-pulse`; `McDonald's Ladestation` (`brand=EWE2Go`, unmatched) becomes `mcdonalds`; restaurant `Total` → `totalenergies`; restaurant `Raststätte Ohligser Heide Ost` with `brand=Aral` → `aral`. Suggest: category sanity check (fuel/charging brands only from `fuel`/`charging_station`/no-amenity, food brands only from gastronomy) and "no name fallback if a `brand` tag exists but is unmatched" (this deviates from heuristics §5 — owner/orchestrator decision).

## F6 — Town fuel stations as Rastanlage

`osm-way-147134992` (unnamed, `A39-SO-0` km 30.8, d 240 m, brands aral/aral-pulse, ATM, car wash) is tagged `highway=services` but is a plain filling station. Together with other unnamed d > 150 m entries (F1) it should be handled by the Autohof classification. No filter can be derived from name; d > 150 m + no Raststätten markers is the best discriminator.

## F10 — Report/pipeline observations

- `report.md` section "Facilities on both sides" lists 116 rows; it should be capped or replaced (F3).
- "Unassigned" list contains Dutch/Belgian objects (`Tienbaan`, `Langveld`, `Bergh Noord/Zuid`, Messeschnellweg): the NRW extract contains border areas; harmless but consider reporting "outside DE" separately. `Raststätte Eltern`, `Herzberg-Ost`, `Raststätte Lastrup Nord/Süd` have no strand at all within 10 km (non-A roads) — consistent with "not on a motorway of the inputs".
- Within-300 m-but-left-of-strand unassigned: 0. This is plausible (no median facilities in the data) but also means the "left" branch of §4.2 is untested on real data.
- `entries()` clamps km with `.min(strand.length_km)` (`facilities.rs:662`) silently; add a counter to the report ("n facilities beyond strand end": 2).
- `unmatched_brands` counts also brands of the services object itself; fine.
- No `autohof` entries yet, so `exit`/`detourKm` are not exercised (0 facilities with `exit != null`).

## Open questions for the orchestrator/owner

1. F1/F2: OK to collapse `highway=services` duplicates and to route Autohof-like objects out of the Rastanlage pass in phase 4 (changes to heuristics §4.2 / new §4.5)?
2. F3: replace the 100 m rule by the geometric criterion (spec §5.2.3 wording "innerhalb von 100 m" would change)?
3. F4: allow `operator` as a brand source for `charging_station` and "HPC brands imply `fastCharger`" (heuristics §5/§6.4)? Brand aliases: Innogy → E.ON Drive, EWE variants, `e-on` spellings, `Total EV Charge`.
4. F7: normalise `Mo-Su 00:00-24:00` variants and/or count 24/7 fuel for `open24h`?
5. F5: is the Dutch `A 7` stub (`A7-O-0`, `A7-W-0`) acceptable in the dataset? Suggest filtering by border polygon or ref plausibility.
6. OSM follow-ups for the owner: `Westpfahlen` typo at Brockbachtal Nord/Süd, `brand=Aral` on restaurant way/136295793, `Brunautal West Ladestation` charger inside Lüneburger Heide West.
