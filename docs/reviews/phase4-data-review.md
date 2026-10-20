# Phase 4 data review (Autohöfe, regional dataset, dataVersion 2026-10-18)

Reviewer: data-reviewer. Scope: `pipeline/out/regional/dataset.json` + `report.md` (HH+NDS+NRW; 130 Rastanlagen, 129 Autohof entries = 68 distinct), `pipeline/src/autohof.rs`, targets `docs/heuristics.md` §7 (V4, D-31, D-32), `docs/schema.md` §2.3, `PLAN.md` D-22…D-32.
No code or data was modified; no internet. OSM links are for the owner's manual check. Everything I state about real-world correctness that needs OSM/web knowledge is marked "unverified".

## Method

- `jq` and `python3 -I` (stdlib) on `dataset.json`: own polyline decoding, own interpolation of the junction position from `km`, own haversine, own recomputation of `detourKm` for **all 129** entries.
- Per-object tags/rules from the read-only variant TSVs (`.../scratchpad/autohof-variants/v0..v5.tsv`, v4 = shipped rule set; v4 has 67 rows, the dataset 68 distinct — the missing one is `way/94520791`, see F2).
- Gates: `rastpipe validate` OK (152 strands, 2240 junctions, 259 facilities); `cargo test --release --offline` 100 passed / 0 failed; `cargo clippy --offline --all-targets -- -D warnings` clean.

## Summary

| # | Sev. | Finding |
|---|---|---|
| F1 | major | Same site emitted twice (3 cases): Autohof + Rastanlage (Westerlinde, 35 m), Autohof twice (Wendhausen / unnamed way, 58 m; Rhüden / "Esso Raststätte Harz", 71 m) |
| F2 | major | One Autohof only in one direction (7 of 68), 3 of them because the opposite-carriageway junction cluster is 1.4–1.9 km away or is named differently, not because the Autohof is one-sided |
| F3 | major | `;` lists in `exit` (2 Autohof entries, 28 junctions in the dataset); the same junction gets swapped value order per direction and therefore different clusters |
| F4 | minor | "One Autohof per cluster" can drop a second real Autohof at the same exit, and picks the unnamed duplicate while a named twin exists (Hamm-Uentrop) |
| F5 | minor | Detour > 2 km on 6 entries (4 distinct), exit names that are rest areas / interchanges on 6 distinct Autohöfe, wrongly attached exit "Raststätte Harz" |
| F6 | minor | Qualification residue: 2 unnamed, 4 bare-brand names ("Shell", "ARAL", "Esso", 2x "tankpool24"), 5 rule-3-only objects, 5 Autohöfe without brands |
| F7 | minor | Recall: `Truck…` fuel stations are not Autohof by rule 2 although `truck` is an Autohof-like keyword for services objects; 6 deferred "Autohof" objects unplaced (> 2 km) |
| F8 | minor (docs) | `schema.md` §2.3 still says Autohof id/name come from the fuel station; D-31 uses the services object |

Positive: all kind-consistency invariants hold (section 4), all 129 `detourKm` values reproduce within 0.15 km of my own interpolation (rounding plus polyline simplification; no outlier), junction km/exit text match the dataset junctions for all 129 entries, no km outside `[0, lengthKm]`, ids unique and well formed.

---

## 1. The 10 report samples, traced

Junction node per strand = `JNode` with minimal haversine distance to the Autohof position among the cluster nodes of that strand (`autohof.rs` entries loop, `per_strand`); `exit` = `Cluster::exit_text()` (`autohof.rs:46`); `detourKm = round(dist_km*1.3, 1).max(0.1)` (`autohof.rs:284`). "own" = my recomputation from polyline interpolation (straight line junction node → position × 1.3). Brands in the 150 m circle agree with the v4 TSV in all 10 cases.

| # | Autohof | Rule (code path) | Cluster / junction nodes | km (O/W…) | straight m → own detour | dataset | Verdict |
|---|---|---|---|---|---|---|---|
| 1 | Truckstop Hamm `node/3550863268` | rule 1 deferred (node, no fuel merged) | A 2 18 Hamm; one node per strand | O 73.586 / W 273.352 | 940 / 926 → 1.2 / 1.2 | 1.2 / 1.2 | plausible (named Truckstop, brands aral, aral-pulse, rewe-to-go) |
| 2 | Oderwald `way/323110477` | rule 1 (D-30 owner: Autohof), fuel way/219740207 merged | A 36 8 Flöthe | N 19.118 / S 18.2 | 565 / 318 → 0.7 / 0.4 | 0.7 / 0.4 | plausible, owner-confirmed; position = fuel station; only brand aral |
| 3 | Autohof Lehrte `way/350011661` | rule 1 + gastro + truck parking | A 2 49 Lehrte | O 263.893 / W 82.479 | 646 / 1110 → 0.8 / 1.4 | 0.8 / 1.4 | plausible; O/W asymmetry real (W node 460 m farther) |
| 4 | Autohof Frechen `way/1252949469` | rule 1 (fuel node/7660932125 merged) | A 1 104 Kreuz Köln-West; **only the NO strand** | NO 55.803 | 712 → 0.9 | 0.9 | position plausible; **SW direction missing** (opposite node 1878 m away in a different cluster, F2) and exit is a Kreuz |
| 5 | Autohof Westerlinde `node/3590608193` | rule 1 (node) | A 39 43 Westerlinde | NO 4.879 / SW 62.026 | 566 / 175 → 0.7 / 0.2 | 0.7 / 0.2 | Autohof plausible, but **duplicate of Rastanlage `way/99351284`** at 35 m (F1) |
| 6 | Autohof Ibbenbüren `way/1193549460` | rule 1 (fuel way/197059377 merged) | A 30 11b Ibbenbüren | O 48.848 / W 86.057 | 310 / 394 → 0.4 / 0.5 | 0.4 / 0.5 | plausible |
| 7 | (no name) `way/28473085` | rule 1 + gastro; no fuel merged | A 2 **57 Lehre** (only O strand) | O 311.328 | 467 → 0.6 | 0.6 | **false/duplicate**: 58 m from Autohof Wendhausen (F1), unnamed |
| 8 | Autohof Rhüden `node/3622838853` | rule 1 | A 7 66 Rhüden/Harz | N 69.375 / S 224.294 | 509 / 247 → 0.7 / 0.3 | 0.7 / 0.3 | plausible; has a second entry "Esso" 71 m away (F1) |
| 9 | (no name) `way/349215236` | rule 1 + gastro; fuel way/1462559112 merged but unnamed | A 2 19 Hamm-Uentrop | O 85.805 / W 261.098 | 170 / 237 → 0.2 / 0.3 | 0.2 / 0.3 | site is real (brands totalenergies, ionity, …) but name null while "Autohof Euro Rastpark Lippetal" (`node/3551312377`, 248 m) exists as the named twin (F4) |
| 10 | Autohof Werne `way/55600480` | rule 1 + gastro + truck parking | A 1 80 Hamm-Bockum / Werne | NO 174.569 / SW 313.654 | 458 / 460 → 0.6 / 0.6 | 0.6 / 0.6 | plausible (exit text with ` / ` is a real OSM name, not a list) |

Result: 7/10 clean, 2/10 duplicates or unnamed doubles of another entry (#5 vs Rastanlage, #7 vs #wendhausen), 1/10 name-less true site (#9). This is in line with the "9/10 real" claim for the *sites* (all 10 are real sites), but 3 of them show defects in how the site is represented.

Mean distance report figure ("466 m") is consistent with my values.

## 2. All 68 Autohöfe

Format: name | exit (strand: km/detour) | entries. Full machine list: reproduce with `jq '[.facilities[]|select(.kind=="autohof")]'`. Flags: **D** = duplicate site, **1E** = single entry, **IC** = exit is Kreuz/Dreieck, **R** = exit is rest area, **>2** = detour > 2 km, **N** = no/generic name, **B** = no brands.

| Name | Exit | Entries | Flags |
|---|---|---:|---|
| Autohof Frechen | 104 Kreuz Köln-West (NO 55.8, 0.9) | 1 | 1E, IC |
| tankpool24 `way/264789294` | 89 Volmarstein (NO 127.1, 0.7; SW 360.6, 1.4) | 2 | N, B |
| Autohof Werne | 80 Hamm-Bockum / Werne (0.6/0.6) | 2 | |
| Autohof Ladbergen | 74 Ladbergen (0.7/0.4) | 2 | |
| Autohof Osnabrück | 71 Osnabrück-Hafen (0.6/0.6) | 2 | |
| Total Autohof Holdorf | 66 Holdorf (0.7/0.4) | 2 | |
| Autohof Motorpark Lohne | 65 Lohne / Dinklage (0.9/0.5) | 2 | |
| Autohof Vechta | 64 Vechta (0.7/0.6) | 2 | |
| Hoyer Autohof Cloppenburger Land | 63 Cloppenburg (0.5/0.6) | 2 | |
| GO Autohof Stuhr | 57 Bremen / Brinkum (0.9/0.9) | 2 | |
| Autohof Bremen | 55 Bremen-Hemelingen (1.0/0.6) | 2 | |
| TotalEnergies Truckstop Oyten | 52 Oyten (0.1/0.2) | 2 | |
| Autohof Zeven-Elsdorf | 48 Elsdorf (0.2/0.4) | 2 | |
| Autohof Sittensen | 47 Sittensen (0.4/0.7) | 2 | |
| Hoyer Autohof Hamburg Süd | 44 Rade (0.3/0.6) | 2 | |
| Truckstop Hamm | 18 Hamm (1.2/1.2) | 2 | |
| (no name) `way/349215236` | 19 Hamm-Uentrop (0.2/0.3) | 2 | N (named twin "Autohof Euro Rastpark Lippetal" lost, F4) |
| Autohof Rheda-Wiedenbrück | 22 Herzebrock-Clarholz (0.4/0.4) | 2 | |
| Aral Autohof Porta Westfalica/Holtrup | 33 Porta Westfalica (0.2/0.4) | 2 | |
| Maxi-Autohof Lauenau | 37 Lauenau (0.6/0.7) | 2 | |
| Truckstop Barsinghausen | 38 Bad Nenndorf (0.8/0.6) | 2 | |
| Autohof Lehrte | 49 Lehrte (0.8/1.4) | 2 | |
| SVG Autohof Schwülper | 53 Braunschweig-Watenbüttel (0.4/0.5) | 2 | |
| MHB Autohof | 55 Kreuz Braunschweig-Nord (1.4/**2.9**) | 2 | IC, >2, B |
| Autohof Wendhausen | 57 Braunschweig-Ost (1.0/0.7) | 2 | |
| (no name) `way/28473085` | 57 Lehre (O 0.6) | 1 | **D** of Wendhausen (58 m), N, 1E |
| Autohof Peine | **52 Peine-Ost;Peine** (W 0.9) | 1 | 1E, `;` |
| Autohof Allermöhe | 2 Hamburg-Allermöhe (0.5/0.4) | 2 | |
| Tankpark Moorfleet | 1 Dreieck Hamburg-Südost (W 0.7) | 1 | 1E, IC |
| Autohof Georgswerder | Hamburg-Elbinsel (S 0.4) | 1 | 1E (opposite node "1 Hamburg Elbinsel / HH-Veddel" is 1457 m away, different name) |
| Autohof Groß-Mackenstedt, Stuhr | 25 Groß Mackenstedt (0.4/0.4) | 2 | |
| Autohof Delmenhorst-Ost | **24;23 Dreieck Delmenhorst;Delmenhorst-Hasport** (NW 1.5) | 1 | 1E, IC, `;` |
| Autohof A28 Apen | 4 Apen / Remels (0.6/0.5) | 2 | |
| Autohof Moorburg | 5 Westerstede-West (1.4/1.5) | 2 | |
| Autohof Dachsberg | 34 Bad Honnef / Linz (0.1/0.3) | 2 | |
| ARAL Truckparking | 4 Rees (0.3/0.3) | 2 | (Autohof Isselburg lost at same exit, F4) |
| Autohof Salzbergen | 7 Rheine-Nord (0.4/0.8) | 2 | |
| Autohof Ibbenbüren | 11b Ibbenbüren (0.4/0.5) | 2 | |
| Autohof Bünde | 27 Bünde (1.2/0.5) | 2 | |
| Schallenkamp | 31 Kreuz Wünnenberg-Haaren (0.3/1.6) | 2 | IC, B, rule 3 only |
| Autohof Paderborn-Mönkeloh | 28 Paderborn-Mönkeloh (0.7/0.8) | 2 | |
| Autohof Hannover Airport | 3 Hannover-Flughafen (0.5/1.0) | 2 | |
| Oderwald | 8 Flöthe (0.7/0.4) | 2 | |
| Autohof Westerlinde | 43 Westerlinde (0.7/0.2) | 2 | **D** of Rastanlage `way/99351284` (35 m) |
| ARAL `way/147134992` | 6 Lüneburg-Nord (0.3/0.6) | 2 | N (bare brand; phase-2 review F1 called it a plain Aral station at A39) |
| Truck Stop Raststätte Düren | 6 Düren (1.0/0.9) | 2 | B, rule 3 only |
| SVG Autohof Köln-Eifeltor | 11b Köln-Eifeltor (O 0.4) | 1 | 1E (A4-W has no junction node within 2.5 km), B |
| Autohof Senden | 3 Senden (0.5/0.5) | 2 | |
| Autohof Soest-Ost | 57 Soest-Ost (0.2/0.6) | 2 | |
| Autohof Geseke | 59 Geseke (0.6/0.1) | 2 | |
| MAXI Autohof Wilnsdorf | 23 Wilnsdorf (0.8/0.5) | 2 | |
| SVG Autohof Reisholz | 25 Düsseldorf-Holthausen (0.4/0.2) | 2 | |
| Autohof Shell Station Gladbeck | 42 Gelsenkirchen-Buer-West (1.2/**2.1**) | 2 | >2 |
| Shell `way/88383867` | 35 Wahn (0.4/0.4) | 2 | N (bare brand), rule 3 only (unverified) |
| tankpool24 `node/8848207454` | 2 Kaldenkirchen (0.3/0.9) | 2 | N, rule 3 only |
| Rosis Leintal Nörten-Hardenberg | 71 Nörten-Hardenberg (0.7/0.4) | 2 | |
| Autohof Northeim Nord | 69 Northeim-Nord (0.6/0.4) | 2 | |
| Esso `way/94520791` | **Raststätte Harz** (N **3.3** / S **2.6**) | 2 | **D** of Rhüden (71 m), R, >2, N |
| Autohof Rhüden | 66 Rhüden/Harz (0.7/0.3) | 2 | |
| Autohof Bockenem | 65 Bockenem (**2.1**/**2.1**) | 2 | >2 |
| Aral Autohof | 62 Hildesheim (0.8/1.0) | 2 | |
| Autohof Mellendorf | 52 Mellendorf (1.9/1.5) | 2 | (close to the 2 km limit, both carriageways) |
| Autohof Schwarmstedt | 50 Schwarmstedt (0.3/0.8) | 2 | |
| Autohof Soltau Süd | 45 Soltau-Süd (0.7/0.7) | 2 | |
| Autohof Soltauer Heide | 44 Soltau-Ost (0.8/1.1) | 2 | |
| Break Autohof Hamburg-Nordheide | 42 Evendorf (0.5/0.6) | 2 | |
| Autohof Altenwerder | 30a Hamburg-Waltershof (0.2/0.9) | 2 | |
| Autohof Lutterberg | 76 Hann. Münden/Staufenberg-Lutterberg (0.5/0.7) | 2 | |

Counts: 61 Autohöfe with 2 entries, 7 with 1 entry (129 = 61*2 + 7). Names missing/generic: 2 unnamed, 4 bare brand/operator names. Without brands: 5 (tankpool24 Volmarstein, MHB Autohof, Schallenkamp, Truck Stop Raststätte Düren, SVG Köln-Eifeltor) — matches "Autohöfe without brands: 5" in the report.
No exit looks like a Rastplatz except the "Raststätte Harz" case. 5 distinct Autohöfe sit on a Kreuz/Dreieck exit (see IC; 7 entries) — those are exactly the cases where the junction node of one carriageway may be a ramp of the interchange rather than a usable exit; owner may want to check Frechen, MHB, Schallenkamp (unverified).

### F1 (major) Same site twice

Evidence (own positions):
- **Westerlinde**: Autohof `osm-node-3590608193@A39-SW-0` (52.12235, 10.22909, km 62.026) and **Rastanlage `osm-way-99351284`** (52.12218, 10.22866, A39-SW-0 km 62.235, name null), 35 m apart, identical brands (shell, tesla-supercharger), both `fastCharger`. The Rastanlage is the unnamed polygon of the same Autohof (phase-2 review F1 already named it the twin of Autohof Westerlinde). It stays a Rastanlage because its `|d|` is <= 150 m so D-22 did not defer it. The user sees "Rastanlage unbenannt" and "Autohof Westerlinde" next to each other on the same strand. Heuristics §7: "Candidates: not assigned to any Rastanlage" is only checked for *fuel* POIs (`autohof.rs` ~l.356 fuel loop), not for deferred services nodes that sit next to a Rastanlage polygon.
- **Wendhausen**: `osm-node-3560180176` (52.31218, 10.63475) in cluster "A 2 57 Braunschweig-Ost" and unnamed `osm-way-28473085` (52.31173, 10.63519), 58 m apart, cluster "A 2 57 Lehre". Both on A2-O-0 (km 310.964 vs 311.328), exit number 57 twice with different names. The report's "kept for nearest site" logic only de-duplicates the same candidate across clusters, not two candidates at the same place.
- **Rhüden**: `osm-node-3622838853` "Autohof Rhüden" (exit 66 Rhüden/Harz) and `osm-way-94520791` "Esso" (exit **Raststätte Harz**, N detour 3.3 km, S 2.6 km), 71 m apart, both entries on A7-N-0 km 69.375 / 67.148. Same physical Autohof (phase-2 review F1 named 94520791 as an unnamed duplicate way of Rhüden). It wins in the rest-area cluster because deferred objects may use `Rast*` clusters (D-32), and loses at Rhüden only by 6 m (247 vs 253 m, `report.md` l.601). The owner example "Esso A 7 Raststätte Harz is an Autohof" may be exactly this site; if so it should appear once, with exit 66.

Suggestion: after the cluster picks, dedupe picks in a radius (suggest <= 150 m = `AUTOHOF_POI_RADIUS_M`), keep the stronger/named/nearer candidate and its junction; additionally exclude deferred nodes/ways within the Rastanlage polygon or <= 150 m of a Rastanlage centroid from being Autohof (or merge the Rastanlage into the Autohof). Expected effect: -3 entries pairs (about -5 entries).

### F2 (major) 7 single-entry Autohöfe

Cause per entry (own junction scan of the same motorway ref within 2.5 km):
- **Frechen**, **Peine**: the opposite-carriageway junction node is 1.88 km / 1.94 km away and belongs to another cluster (>1.5 km apart, or different `ref;name` ordering). Both clusters list the Autohof within 2 km, but the rule "a candidate that is the nearest of several sites stays with the nearest site only" (`autohof.rs`, `kept` loop; report.md l.527 "kept for the nearer cluster A 1 104 Kreuz Köln-West (714 m)") drops the opposite direction. Result: drivers in the other direction never see Frechen / Peine. This is a defect for any Autohof at a Kreuz or at an exit with widely separated ramps.
- **Peine**: additionally OSM writes `52 Peine-Ost;Peine` on one carriageway and `52 Peine;Peine-Ost` on the other; the cluster key `(ref,name)` therefore splits one Anschlussstelle (F3).
- **Delmenhorst-Ost**: SO nodes `23 Delmenhorst-Hasport` (2.4 km) and `24 Dreieck Delmenhorst` (1.9 km) are separate junctions; NW carries the combined `24;23` list.
- **Lehre/Wendhausen**: artefact of F1.
- **Georgswerder** (A255 S, node name without ref vs N "1 Hamburg Elbinsel / HH-Veddel", 1457 m) and **Moorfleet** (A25-W only; the east carriageway has no junction node in 2.5 km): possibly really one-sided; unverified.
- **Köln-Eifeltor**: A4-W has no junction node in 2.5 km (OSM gap or the Autohof is only reachable from O).

Suggestion: when the nearest-site rule would drop a cluster, keep it if the cluster is on a *different strand of the same motorway ref* and within 2 km (the 2 km candidate limit already applies), i.e. apply the "nearest site" rule per strand instead of globally. Normalise the cluster key (sorted split of `ref`/`name` on `;`, F3).

### F4 (minor) One Autohof per cluster

`report.md` "Additional candidates" (85) contains genuine Autohöfe that lose only because another candidate at the same exit is stronger/nearer:
- `Autohof Isselburg` (`node/3561599056`, A 3 4 Rees, 396 m, rule 2) lost against `ARAL Truckparking` (192 m, rule 1). Probably the same site as `way/162474435` "ARAL Tankstelle Isselburg" (195 m) but not verified; if not, a second real Autohof is missing.
- `Autohof Eicker` (`node/3550691614`, A 1 80 Hamm-Bockum/Werne, 503 m) lost against Autohof Werne (458 m): two different Autohöfe at the same exit (unverified).
- Named twins lost against an unnamed deferred object: `Autohof Euro Rastpark Lippetal` (248 m) vs unnamed `way/349215236`; `Autohof Schwarmstedt node/3620418627` (364 m) vs `way/356620997`; `MBH Autohof way/350071160` vs `MHB Autohof node/3558214615` (typo duplicates, 2 objects, 1 site). An unnamed pick whose merged fuel station is unnamed should inherit the name of the nearest named Autohof-keyword candidate within 150–250 m (or apply the F1 dedupe with name inheritance).

### F5 (minor) Detour, interchanges, rest-area exit

- Detour > 2 km: 6 entries (4 distinct): MHB Autohof W 2.9 (E 1.4), Esso/Raststätte Harz N 3.3 and S 2.6, Autohof Bockenem 2.1 and 2.1, Shell Gladbeck SW 2.1. `detourKm` is the straight line to the carriageway-specific junction node × 1.3, so values above 2 km mean the *opposite* node is > 1.5 km away. Plausible but suspicious; Bockenem (both 2.1) and Mellendorf (1.9/1.5) sit at the 2 km candidate limit and may have a nearer real exit that OSM does not tag (unverified).
- Max detour in the report is stated as 3.3 km, which matches.
- Interchange exits (7 entries/5 distinct): Frechen, MHB, Schallenkamp, Tankpark Moorfleet, Delmenhorst-Ost. Not wrong per heuristics §7 (clusters are named by OSM), but for a driver an Autohof "at Kreuz …" is only usable if the Kreuz has an exit. Recommendation: owner/unverified.
- "Raststätte Harz" as exit text: only the F1 duplicate.

### F6 (minor) Qualification residue (D-32)

- Rule-3-only objects (5): tankpool24 `way/264789294` (89 Volmarstein, 555 m) and `node/8848207454` (2 Kaldenkirchen, 239 m; both card-only truck fuel network, `hgv=yes` parking), Schallenkamp `node/279395139` (236 m), Shell `way/88383867` (35 Wahn, 327 m, McDonald's-type gastro within 150 m, brand-less otherwise), Truck Stop Raststätte Düren `node/3572182602`. At least Wahn looks like a town station next to a truck parking; marginal. All five are within the stated rule, not rule violations.
- `ARAL` `way/147134992` (A 39 6 Lüneburg-Nord): qualifies via rule 1 because |d| > 150 m; phase-2 review already doubted it ("plain Aral station"). Brands in circle: mcdonalds, aral, aral-pulse. Unverified.
- Names: 4 bare operator/brand names will show as "Shell", "ARAL", "Esso" in the list; the app could append the exit ("Shell – Wahn") — display decision, not pipeline.

### F7 (minor) Recall

V0 (spec rule) placed objects with `Truck`/`Autohof` in name / operator that are absent from the dataset (searched all six TSVs): only
- `Total Energies Truckstop` `way/1237315739`, A 1 56 Bremen-Arsten, 1425 m: rule 3 requires <= 1 km to the junction (and truck parking), rule 2 requires keyword "Autohof".
- `Truck Service` `node/339322584`, A 44 21 Mönchengladbach-Ost, 1004 m: just over the 1 km limit.
- Hoyer Tank-Treff `way/537581568` (A2 54 Braunschweig-Hafen, 191 m), `Hoyer` `way/1135160350` (A28 10 Oldenburg-Wechloy), `Hoyer Turbo-Tank Delmenhorst`: gastronomy but no truck parking within 300 m (rule 3 fails) — fine.
`AUTOHOF_LIKE_KEYWORDS = ["autohof","truck","maxi"]` (`params.rs:28`) applies only to deferring services objects, rule 2 knows only "Autohof"; so "TotalEnergies Truckstop Oyten" qualifies via a deferred node while "Total Energies Truckstop" Arsten does not. Suggestion: owner decision whether rule 2 should include `truckstop|truck stop`.
Report "Unplaced Autohof-like services" (6, > 2 km from every junction node): Autohof Diepholz `way/64199695`, Autohof Hansator `way/114309735`, K1 Autohof `way/220944902`, Raiffeisen Autohof `way/242810362`, Autohof Hamburg-Moorfleet `way/347955640` (near Tankpark Moorfleet, so a duplicate), Fehlhaber Aral-Autohof `way/1274096743`. These are real-looking Autohöfe at B-roads/not within 2 km of an A-road junction; consistently dropped, documented in the report. OK.

Why the objects from the three groups are missing: rule 3 distance (1 km), rule 2 keyword spelling, or the 2 km candidate limit; none is an implementation bug.

### F8 (docs) Schema drift

`docs/schema.md` §2.3 (`id`, `name`, `lat/lon` rows) says id/name come from the fuel station and the position is the fuel station. D-31 / heuristics §7: id/name of the services object (fallback fuel station), position = fuel station if present else centroid. The code follows D-31. Proposal for the owner/architect: schema.md `id`: "`type`/`osmId` of the services object, else of the fuel station"; `name` likewise; `lat/lon`: "fuel station if present, else centroid".

---

## 3. Exit text formatting (`;` lists)

Counts (all from `dataset.json`):
- Facility entries with `;` in `exit`: **2** (both Autohof, 1 entry each): `52 Peine-Ost;Peine` (Autohof Peine, A2-W), `24;23 Dreieck Delmenhorst;Delmenhorst-Hasport` (Autohof Delmenhorst-Ost, A28-NW).
- Junctions (`junctions[]`) with `;` in `ref` or `name`: **28** of 2240 (25 in `name`, 11 in `ref`; 3 have a ref list and no name, e.g. A52-NO-0 km 1.268 `21;22`). Also `ref` "8 (19)" (1x) is another raw OSM value.
- The same junction appears with swapped order per carriageway: `52 Peine;Peine-Ost` (A2-O) vs `52 Peine-Ost;Peine` (A2-W); `16 Kreuz Oldenburg-Ost;Oldenburg-Osternburg` (NW) vs `16 Oldenburg-Osternburg;Kreuz Oldenburg-Ost` (SO); `46;47 …` / `34;33 …`. Since `(ref,name)` is the cluster key (`autohof.rs` `build_clusters`), the two carriageways of such an exit are **different clusters** (contributes to F2 Peine).

Proposed display rule (implement in the pipeline `Cluster::exit_text`, schema §2.3 says `exit` is display text; junction `ref`/`name` in §2.x likewise):
1. Split `ref` and `name` on `;` and trim.
2. Cluster key: sorted lists (so both carriageways merge into one cluster).
3. If `|refs| == |names|`: pair them, join with `" / "` and no repetition: `34;35 Dreieck Hamburg-Südost;Hamburg-Moorfleet` → `34 Dreieck Hamburg-Südost / 35 Hamburg-Moorfleet`; `24;23 Dreieck Delmenhorst;Delmenhorst-Hasport` → `24 Dreieck Delmenhorst / 23 Delmenhorst-Hasport`.
4. If counts differ or one side is missing: `refs.join("/")` + " " + `names.join(" / ")`: `52 Peine-Ost;Peine` (one ref, two names) → `52 Peine-Ost / Peine`; `5 Essen;Gladbeck` → `5 Essen / Gladbeck`; `21;22` → `21 / 22`; `34;33 Kreuz Braunschweig-Süd/Braunschweig-Südstadt` → `34 / 33 Kreuz Braunschweig-Süd/Braunschweig-Südstadt` (ugly but rare; compact form below)
5. Compact alternative (first values only) for the one-line list in the app: `52 Peine-Ost`, `24 Dreieck Delmenhorst`, `5 Essen`. Not recommended as the only rule: it is order-dependent (`52 Peine` vs `52 Peine-Ost` per direction) unless step 2 sorts first.
Recommendation: rule 1–4 (pairwise), with sorting in the cluster key so that both directions show the same text. Existing real names with `/` in the OSM string ("Bremen / Brinkum", 93 junctions) are not lists and must stay untouched; only `;` is the OSM list separator.

## 4. Kind consistency (all hold)

- 259 entries = 130 `rastanlage` + 129 `autohof`; `kind` only these two values.
- Autohof: `exit` non-null and non-empty for all 129; `detourKm > 0` for all (min 0.1, no zero); id matches `^osm-(node|way|relation)-[0-9]+@.+$`, suffix equals `strandId` for all 129 (0 mismatches).
- Rastanlage: `exit == null` and `detourKm == 0` for all 130.
- Ids unique (0 duplicates). No OSM object (id part before `@`) is both Rastanlage and Autohof: intersection empty. No `km` outside `[0, lengthKm]`; all `strandId` exist.
- Autohof position vs Rastanlage: only **one** Autohof lies < 1 km from any Rastanlage centroid: Westerlinde at 35 m (F1). I could only use Rastanlage centroids (the dataset has no polygons; the polygon check on `autohof.rs` fuel POIs is implemented there — `fuel_in_rastanlage_polygon 22` in the report), so a deferred *node* inside a Rastanlage polygon (which Westerlinde appears to be) is not covered by that check.
- Distance of the Autohof position to its own strand axis: min 30 m (Gladbeck, 42), 45 m, 48 m (Delmenhorst-Ost), 55 m (Westerlinde); all others > 60 m. These four are adjacent to the carriageway; none is on it.
- Junction km/exit text: all 129 entries have a dataset junction on the same strand at the same km (±1.5 m) whose `ref name` composition equals `exit`.

## 5. Filter acceptance and fixture strands

- `jq '[.facilities[]|select(.kind!="rastanlage")]|map(.kind)|unique'` → `["autohof"]`; `select(.kind!="autohof")` → `["rastanlage"]`. So kinds=[rastanlage] leaves 130 entries and no Autohof; kinds=[autohof] leaves 129 and no Rastanlage; both = 259. IDs are clean (previous section).
- AheadQuery-relevant facilities (km in (from, to] on the fixture strand), sorted by km:
  - Fixture 01 `A1-SW-0` 16.75→106.0: Autohof Hoyer Hamburg Süd (40.268, exit 44 Rade, 0.6) · Rast Hollenstedt (45.118) · Rast Ostetal Nord (60.566) · Autohof Sittensen (62.105, 0.7) · Autohof Zeven-Elsdorf (75.423, 0.4) · Rast Grundbergsee Nord (95.084). Oyten (106.013) is 13 m behind the end. Order and exits sane: exit numbers 44 (km 40.3) < 47 (62.1) < 48 (75.4) increase with km.
  - Fixture 02 `A1-NO-0` 382.3→471.4: Oyten (382.34, 0.1) · Rast Grundbergsee Süd (393.121) · Rast Aral (406.977) · Zeven-Elsdorf (412.661, 0.2) · Sittensen (425.914, 0.4) · Rast Ostetal Süd (427.713) · Rast Aarbachkate (443.143) · Hoyer Hamburg Süd (447.783, 0.3). Sane; a facility returned for the opposite direction fixture 01 would show different ids (`@A1-SW-0`), none leaks across strands (`strandId` filter).
  - Fixture 05 leg 1 `A1-NO-0` 446.2→461.2: Hoyer Hamburg Süd (447.783). Leg 2 `A7-S-0` 36.15→51.15: no facilities. Sane (short legs through Kreuz Hamburg-Süd).
  - Fixtures 03/04: strand A1-SW-0 47–67 km (notOnMotorway) and 20–25 (tooSlow): not relevant for Autohof results (the 03 range contains Sittensen and Rast Ostetal Nord, which must stay unreturned because the fix is `notOnMotorway`).
- Remark for the Swift AheadQuery tests: kinds filter works purely on `kind`, the fixture-02 acceptance clause "every returned `rastanlage` right of the strand" does not apply to Autohöfe (they are on the junction side, `km` = junction km, `lat/lon` = fuel station). Autohof `km` is the junction km, so for Autohöfe like Lehrte-W (detour 1.4) the app announces the *exit*, not the Autohof position; fine per schema.

## 6. Coverage: real Autohöfe missing from the dataset

See F7 and F4. Complete list I can derive from the TSVs and the report (names containing Autohof/Truck/Maxi/Hoyer, absent from the 68):
- Absent because of rule 3 distance/keyword: Total Energies Truckstop (Arsten), Truck Service (Mönchengladbach-Ost); Hoyer Tank-Treff, Hoyer (Oldenburg-Wechloy), Hoyer Turbo-Tank Delmenhorst (no truck parking, intentionally dropped by V4).
- Absent because of "one Autohof per cluster" or duplicate-of-picked: Autohof Sittensen `node/3542183071` and Hoyer Autohof Hamburg Süd `node/3624017502` (same sites as picked ways, ok), Autohof Eicker, Autohof Euro Rastpark Lippetal, Autohof Isselburg, MBH Autohof (way), Autohof Schwarmstedt (node), plus ~20 non-Autohof fuel stations that rightly lost.
- Absent because > 2 km from every junction: the 6 "Unplaced Autohof-like services".

Why Autohöfe are missing in general: the heuristic only considers fuel stations/services within 2 km of a *motorway_junction node*; every Autohof that is more than 2 km from an A-road junction or that is attached to a federal road (B 51 Diepholz, etc.) disappears, by design.

---

## Open questions for the owner (unverified items)

1. Is "Esso" at `way/94520791` (exit "Raststätte Harz") the same Autohof as Autohof Rhüden (OSM check)? If yes, F1 dedupe removes it; if not, it needs exit 66 or a rest-area exit consistent with the Rastanlagen.
2. Should the opposite carriageway always get an entry when the cluster of that direction is within 2 km (F2)? Recommended yes.
3. Display rule for `;` exits (section 3): choose 1–4 (pairwise, ` / `) vs first-value-only.
4. Should rule 2 accept `Truck(stop)` fuel stations (F7)?
