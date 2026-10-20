# Heuristics (pipeline)

Algorithms of the dataset pipeline (`rastpipe`, Rust), with rationale. Section numbers match the
heuristics document of the app repository (which keeps §8 MapMatcher, §12 drive mode and §13
dataset update). Geometry conventions (earth radius, sign of `d`, projection) are in
`docs/schema.md` §3 and apply here without exception.

Marked with **[Decision]** = specification by the maintainer where the original assignment left
things open. All parameters are named constants in `pipeline/src/params.rs` with the values from
this document. References like `PLAN D-22` point to the decision log of the app repository.

---

## 1. Input and two passes (spec 5.1.1)

- **Pass 1** (one scan over the PBF, including `DenseNodes`) collects:
  - Ways `highway=motorway` with `ref` containing at least one A number (§2).
  - `highway=services`: closed ways, relations `type=multipolygon` (members with role `outer`
    or empty role), nodes.
  - Nodes `highway=motorway_junction`.
  - POIs (nodes **and** closed ways; ways via their centroid) with `amenity` ∈ {`fast_food`,
    `restaurant`, `cafe`, `fuel`, `charging_station`, `shower`} or with a `brand` tag.
  - Required node IDs of all collected ways/relations go into a `HashSet<i64>`.
  - **[Decision]** Relations reference ways that may come *after* the relation in the file
    (or before). Hence: in pass 1, also store the member way IDs of `services` relations and the
    node lists of **all** closed ways that are member candidates. Cheaper alternative, if memory
    gets tight: a third pass only over ways. pipeline-dev decides by measurement and reports.
- **Pass 2** loads coordinates of exactly the remembered node IDs.
- Memory: peak RSS (`/proc/self/status` → `VmHWM` on Linux, `getrusage` `ru_maxrss` on macOS,
  without a crate via `std::fs` or `libc`-free parsing; on macOS "n/a" is acceptable) and
  runtime per pass go into the report.
- **[Decision] Multiple regions:** `--input` may be given multiple times. All inputs are read
  in sequence in both passes; objects are deduplicated by (type, OSM ID) (Geofabrik extracts
  overlap at the borders and contain identical objects). No external tool (osmium) needed.

## 2. Parse ref (spec 5.1.2)

- Split `ref` at `;`, trim each part.
- A part counts as an A number if it matches `^A\s*(\d+)$` (case-insensitive). Normalize to `A <n>`
  (leading zeros removed).
- E numbers and everything else are ignored. Ways without an A number are discarded.

## 3. Strands (spec 5.1.3–5.1.6)

### 3.1 Direction
- `highway=motorway` is implicitly one-way: direction = node order of the way.
- `oneway=-1` → reverse the node order. `oneway=no` → way goes in **both** directions (two
  directed edges). All other `oneway` values (`yes`, missing, …) → as-is.

### 3.2 Chaining (per A number)
- Graph: vertices = OSM node IDs at the way ends, edges = directed ways (length = Haversine sum).
- **[Decision] Algorithm (deterministic):**
  1. For every unused edge compute `down(e)` = length of `e` + max `down` over all unused
     successor edges (memoized DFS; cycle guard: an edge already on the current DFS stack
     counts as a non-existent successor).
  2. Start edge = the unused edge without an unused predecessor that has the largest `down`;
     if all edges have predecessors (pure cycle): edge with the largest `down`. Ties: smaller
     way ID.
  3. Follow the chain: at each vertex take the successor with the largest `down` ("the longer
     continuation wins"). Ties: smaller way ID. Mark edges as used.
  4. Recompute `down` (only for affected edges) and repeat until all edges are used.
  5. Chains > 2 km → strand. Chains ≤ 2 km → report section "Discarded remnants" (ref, length,
     way IDs).
- **U-turn guard:** a successor whose first segment reverses the bearing of the last segment by
  more than 150° is not considered a continuation (prevents `oneway=no` ways from merging both
  directions of travel into one chain).
- Rationale: the greedy longest path reproduces the "main carriageway" and makes branches at
  interchanges (e.g. carriageway splits) separate strands instead of leaving gaps.

### 3.3 Direction families and IDs
- `towards` from the initial bearing first → last point (schema §2.1).
- Both carriageways of a motorway emerge automatically as separate chains with opposing
  bearings; there is no explicit pairing in the dataset.
- **Opposite direction** (needed for invariants, §4.4): strand `B` is the opposite direction of
  `A` at position `P` if `B.ref == A.ref`, `B ≠ A`, and the local segment bearing of `B` at the
  projection of `P` differs from that of `A` by > 135°. (Local instead of `towards`, because long
  strands curve.)

### 3.4 Simplification and km
- Douglas–Peucker with 15 m tolerance in local metric coordinates (perpendicular distance to
  the segment). Polyline round-trip: km values are computed **after** encoding on the decoded
  points (schema §3).
- Junctions are projected onto the **simplified** strand (schema §2.3). A junction node belongs
  to strand `S` if it is a node of one of the ways of `S`. If it lies on several strands (shared
  carriageway, `A 1;A 3`), it is emitted once per strand.

## 4. Rastanlage → strand (spec 5.2)

### 4.1 Centroid
- Polygon: area-weighted centroid (shoelace) of all outer rings in local coordinates.
- Node: the node position.
- Degenerate polygons (area ≈ 0): mean of the vertices.

### 4.1a Exclusions and merging before assignment (owner OK, P-01/P-02)
- **[D-33] Order:** duplicates are merged **first**, the Autohof-like exclusion is evaluated on the
  merged object (all names/operators/brands of the group count). A site whose node says "Autohof"
  is therefore an Autohof as a whole, never a Rastanlage plus an Autohof.
- **Autohof-like services objects are not Rastanlagen.** A `highway=services` object is excluded
  from the Rastanlage pass if `name`, `operator` or `brand` contains `Autohof`, `Truck` or `Maxi`
  (case-insensitive, also inside words, e.g. "Truckstop", "MAXI Autohof"), **or** its centroid is
  more than **150 m** from the chosen strand (`|d| > 150 m`). Excluded objects are listed in the
  report section "Autohof-like services (deferred to phase 4)" and are input to the Autohof logic
  (§7) from phase 4 on.
- **Duplicates are merged.** Two remaining services objects are the same site if they would be
  assigned to the same strand, their km differ by ≤ **0.35 km**, and the centroid of one lies inside
  the polygon of the other. Merge transitively. Kept object: the polygon (two polygons: the larger
  area; ties: smaller OSM type/id). From the dropped objects, `name`, `brand`, `shower` and
  `opening_hours` fill in tags that the kept object lacks. The kept object's ID is the facility ID.
  Merging happens **before** POI assignment. Report section "Merged duplicates" lists them.

### 4.2 Choice of strand
- Candidates: strands whose nearest segment is ≤ **300 m** from the centroid.
- Per candidate, the signed `d` relative to the nearest segment. Valid if `d > 0` (right).
- Chosen: the valid candidate with the smallest `|d|`. None valid → report "Unassigned
  facilities" (name, OSM link, nearest strand, `d`), not included.
- `km` = projection of the centroid.

### 4.3 Facilities on both sides (spec 5.2.3)
- **[Decision, owner OK, P-03; deviation from spec §5.2.3]** A facility counts as "on both sides" if,
  relative to an opposite strand (§3.3) within 100 m, **any** of these holds: its polygon crosses the
  opposite strand; a polygon vertex has `d_own < −15 m` (reaches beyond its own carriageway); or the
  centroid is nearer to the opposite strand than to its own. Node facilities: only the last rule.
  → report section "Facilities on both sides". Assignment is still made purely by the right-hand rule.
- Rationale: carriageways are only 2–28 m apart (median 15 m); the spec's plain 100 m rule flagged
  116 of 176 facilities, the geometric rule flags only genuine cases (phase 2 review F3).

### 4.4 Shared sections (`A 1;A 3`)
- **[Decision, owner OK]** On sections with several A numbers there are several geometrically
  identical strands (e.g. `A1-…` and `A3-…`). Otherwise whoever is matched on the other strand
  would never see the facility. Rule: among the valid candidates (§4.2), every candidate whose
  `ref` differs from the chosen one and whose `|d|` is within **5 m** of the chosen `|d|` also gets
  the facility (at most one strand per `ref`, the one with the smallest `|d|`). The facility is
  then emitted once per such strand with ID `<baseId>@<strandId>` (schema §2.3); `km` is the
  projection onto the respective strand. Corridor and amenities are computed once on the chosen
  (nearest) strand and copied. Report section "Shared sections" lists these facilities.

## 5. Brand matching (spec 5.4)

- Normalization for all comparisons: trim, collapse whitespace sequences into one space,
  Unicode lowercase, typographic apostrophes `’ʼ` → `'`.
- **brand tag:** split at `;`. Each part is compared for **equality** with all `match.brand`
  entries. Hits → brand IDs.
- **name** only if the `brand` tag is missing or none of its parts matched: a `match.name`
  pattern matches if it occurs in `name` as a **whole word sequence** (word boundary =
  string start/end or a non-letter/digit character). If several patterns match overlapping text,
  only the longer one counts (`Shell Recharge` beats `Shell`).
- **[Decision, owner OK, P-04] Chargers:** for POIs with `amenity=charging_station` and no matching
  `brand`, the `operator` tag is matched like a `brand` tag (equality per `;` part, then whole-word
  name patterns). Other POI types never use `operator`.
- Sources of brands of a Rastanlage: all POIs assigned to it (§6) plus the `highway=services`
  object itself (its `brand` tag only, not `name`, because names like "Rasthof Aral" are rare
  and `name` is the location name).
- Unmatched `brand` values of assigned POIs → report with frequency (descending).

## 6. POI corridor and amenities (spec 5.3, 5.4)

### 6.1 Corridor (polygon facilities)
- Projection of all outer-ring vertices onto the **assigned** strand. **[Decision]** The
  projection of vertices is restricted to the strand section ±2 km around `km` of the centroid
  (prevents jumps onto distant parts of a curving strand).
- `s_min, s_max` = min/max of the vertex `km` values. `d_min, d_max` = min/max of the vertex `d`
  values.
- POI belongs to the facility if it is **in the polygon** (ray casting on the outer rings) or
  `s_min ≤ s_poi ≤ s_max` and `d_min ≤ d_poi ≤ d_max + 50 m`.
- `d_min` can be ≤ 0 if the polygon touches the carriageway or overlaps it; the invariant §6.3
  limits that.

### 6.2 Corridor (node facilities)
- `s ∈ [s_node − 0.150 km, s_node + 0.150 km]`, `d ∈ [max(0, d_node − 30 m), d_node + 50 m]`.

### 6.3 Invariant (hard)
- **[Decision]** The invariant is checked **in two stages**:
  1. **Filter during assignment:** a POI candidate (also inside the polygon!) is only assigned if
     `d_poi > 0` and its nearest strand (over all strands, by distance) is **not** an opposite
     direction (§3.3) of the facility strand. Excluded candidates → report section "POIs excluded
     by invariant" (no violation).
  2. **Assertion after the build:** independent check of all final assignments with the same
     rule, computed separately. Each hit is an "invariant violation" → `--strict`: exit code ≠ 0
     and no output files; otherwise report entry.
- Rationale: polygons that are drawn too large (over the carriageway) must not pull POIs of the
  opposite side in; at the same time stage 2 detects programming errors in the assignment.
  Acceptance "0 violations" refers to stage 2.
- A POI assigned to several facilities: assign only to the one with the smallest `|d_poi − d_centroid|`
  (rare, report).

### 6.4 Amenities
- **Gastronomy POI** = `amenity` ∈ {`fast_food`, `restaurant`, `cafe`}.
- **shower:** `amenity=shower` in the corridor, or `shower=yes` on the facility or on an
  assigned POI.
- **fastCharger:** an assigned `amenity=charging_station` with
  - `socket:type2_combo` as an integer ≥ 1 (values like `yes` don't count), or
  - a `socket:*:output` or `maxpower` value ≥ 50 kW, or
  - **[P-04]** a resolved brand in the fast-charging set `ionity`, `tesla-supercharger`, `fastned`
    (named constant `FAST_CHARGER_BRANDS`).
  - Power parsing: split the value at `;`, per part: number (decimal point or comma) + optional
    unit `kW`/`W`/`kVA`/`MW` (case-insensitive, with or without a space). Without a unit: ≥ 1000
    → watts, otherwise kW. Maximum of all parts. Unparseable → ignore.
- **open24h:** `opening_hours` equals `24/7` on the facility or on at least one assigned
  gastronomy POI. **[P-05, owner OK]** Comparison after removing all whitespace, case-insensitive;
  equivalent spellings: `24/7`, `Mo-Su 00:00-24:00`, `Mo-Su 0:00-24:00`, `00:00-24:00`. Fuel and
  other non-gastronomy POIs do **not** count (owner: not the app's focus). Nothing else is parsed.

## 7. Autohöfe (spec 5.5, phase 4)

- **Junction clusters:** a junction often consists of one `motorway_junction` node per
  carriageway. **[Decision]** Nodes with the same motorway ref, the same (`ref`, `name`) and
  a distance ≤ 1.5 km form a cluster. Clusters = "Anschlussstelle".
- **Candidates:** `amenity=fuel` (node or way centroid), not assigned to any Rastanlage, ≤ 2 km
  straight-line distance to the nearest junction node.
- **[Decision, D-31] Deferred services objects** (§4.1a: Autohof-like or |d| > 150 m) are candidates
  too, positioned at their centroid, and **automatically count as Autohof**. A fuel candidate whose
  position lies inside such a services polygon (or ≤ 150 m from a services node) belongs to that
  services object: one candidate, `type`/`osmId`/`name` of the **services object** (fallback: the
  fuel station's name), position = fuel station if present (more precise for navigation), else the
  centroid. Deferred objects farther than 2 km from every junction → report "Unplaced Autohof-like
  services".
- **[Decision D-32, owner OK — replaces the spec 5.5 qualification]** A candidate is an Autohof if
  1. it is a deferred services object (§4.1a; always qualifies), **or**
  2. `Autohof` (case-insensitive, also inside words) occurs in `name`, `operator` or `brand`, **or**
  3. a gastronomy POI is ≤ 150 m away **and** a truck parking is ≤ **300 m** away **and** the
     straight-line distance to the nearest junction node of the cluster is ≤ **1 km**.
     Truck parking = `amenity=parking` with `hgv` ∈ {`yes`, `designated`, `only`, `official`} or
     `capacity:hgv` > 0 (pass 1 collects these parkings).
  Rationale: the spec rule (gastronomy ≤ 150 m alone) produced 556 Autohöfe in HH+NDS+NRW, 9 of 10
  samples were ordinary town fuel stations. Rule 3 follows the VEDA Autohof criteria (≤ 1 km from
  the exit, truck parking). Measured: 67 Autohöfe, 9/10 samples real (phase 4 variant analysis).
- **Rest-area exits:** junction clusters whose name starts with one of the words `Rastplatz`,
  `Raststätte`/`Raststaette`, `Rasthof`, `Rastanlage` (case-insensitive; not a bare `Rast` prefix,
  which would hit the real exit "Rastede") are not Anschlussstellen. They are skipped for candidates
  of rules 2 and 3; deferred services objects (rule 1) may still use them (owner: "Esso A 7
  Raststätte Harz" is an Autohof).
- **[D-33] Junction cluster key normalization:** `ref` and `name` are split at `;`, trimmed,
  sorted and re-joined before clustering (`52 Peine-Ost;Peine` ≡ `52 Peine;Peine-Ost`).
- **[D-33] Keyword:** rule 2 also accepts `Truckstop` / `Truck Stop` (same keywords as §4.1a minus
  `Maxi`).
- **[D-33] Nearest site per strand:** the "keep a candidate only for its nearest cluster" rule is
  applied **per strand**, so each carriageway gets its own nearest exit (fixes missing
  opposite-direction entries).
- **[D-33] Site dedupe:** after picking, Autohof entries on the same strand whose positions are
  ≤ **150 m** apart (or whose OSM objects are merged duplicates) are one site; keep the entry with
  the smallest `detourKm` (ties: smaller OSM type/id). An Autohof position ≤ 150 m from a
  Rastanlage centroid is dropped (report).
- **Per cluster** the strongest candidate wins (rule 1 > rule 2 > rule 3), ties by distance to the
  cluster (minimum distance to one of its nodes). Others → report "Additional Autohof candidates".
- **Entries:** one per strand on which a node of the cluster lies (usually 2). Per entry:
  `km` = junction-km on this strand, `exit` per schema, `detourKm` = straight-line distance
  junction node of this strand → fuel station × 1.3.
- **Brands/amenities:** from all POIs ≤ 150 m around the fuel station (circle, not a corridor;
  there is no carriageway in between because an Autohof lies outside). Same rules as §5/§6.4.
- `--no-autohoefe` skips this step completely.

## 9. report.md (spec 5.6)

Sections in this order (Markdown, headings level 2):
1. Summary (inputs, dataVersion, strands: count + total length per ref, facilities by `kind`,
   brand frequency)
2. Invariant violations (count first; 0 → "None")
3. Unassigned facilities
3a. Autohof-like services (deferred to phase 4)
3b. Merged duplicates
4. Facilities on both sides
5. Shared sections
6. POIs excluded by invariant
7. Unmatched brands (value, frequency)
8. Discarded remnants
9. Autohöfe (phase 4: counts, additional candidates, 10 samples)
10. Memory and runtime
11. Samples: 20 random facilities (fixed seed `20261019`, own simple PRNG e.g. SplitMix64)
    with name, kind, strand, km, brands, amenities, OSM link
    (`https://www.openstreetmap.org/<node|way|relation>/<id>`)

## 10. Explicitly not covered (MVP)

- `highway=rest_area` (car parks without services) is not collected.
- No transitions across interchanges: at the end of a strand the list ends.
- Bridge restaurants on both sides: no special logic (only the report, §4.3).
- Gaps in the `ref` tagging (e.g. ways without `ref` in tunnels) split strands. Visible in the
  report via strand count/length; no gap closing in the MVP.
- Live Activities, background location and announcements were excluded from the MVP; phase 7
  adds them (§12, PLAN D-37).

## 11. Test drives (GPX fixtures, phase 2)

Contract between `rastpipe gpx` (writer) and the RastCore replay tests (reader). **[Decision]**

### 11.1 GPX file
- GPX 1.1, only `<wpt lat=".." lon=".."><time>…</time></wpt>` elements in travel order (the format
  Xcode's location simulation reads). No extensions.
- lat/lon with 6 decimal places, `time` RFC 3339 UTC, start `2026-01-01T00:00:00Z`, **1 point per
  second**.
- Geometry: walk along the decoded strand from `fromKm` to `toKm` at constant `speedKmh`
  (`fromKm > toKm` is not allowed — reverse direction = the other strand). Lateral offset
  `offsetM` with the sign of schema §3 (`> 0` = right of the travel direction). Noise: per point
  independent, isotropic Gaussian with standard deviation `noiseM` (Box–Muller over an own
  seeded PRNG, e.g. SplitMix64). Same arguments + seed → byte-identical file.
- **Legs:** `--strand/--from-km/--to-km` may be given several times (in this order, as groups);
  the legs are concatenated without a time gap (one interchange transition). This is a
  **deviation from the spec CLI** (repeatable instead of single) and is backwards compatible.

### 11.2 Sidecar `<name>.json` next to each GPX
```json
{
  "name": "01-a1-harburg-bremen",
  "seed": 1, "speedKmh": 120, "offsetM": 0, "noiseM": 8, "intervalS": 1,
  "legs": [ { "strandId": "A1-SW-0", "fromKm": 3.2, "toKm": 95.0, "firstPoint": 0, "lastPoint": 2789 } ],
  "expect": "matched"
}
```
- `expect`: `matched` (points should be matched to the leg's strand), `notOnMotorway`, `tooSlow`.
- `firstPoint`/`lastPoint`: inclusive point indexes of the leg in the GPX.

### 11.3 Replay evaluation (RastCore tests)
- Course is derived from the GPX: bearing from point `i−1` to `i+1` (centered), first/last point
  one-sided. **Speed = the nominal `speedKmh` of the sidecar** (CoreLocation reports Doppler speed,
  which is far less noisy than a position-derived one; deriving it from 8 m noisy points scatters it
  by ±5 m/s and makes the slow-drive fixture meaningless). Accuracy = 10 m constant.
- Acceptance (spec §6 phase 2):
  - `matched` fixtures: ≥ 95 % of the points (speed ≥ 40 km/h) are `.matched` on the
    leg's strand. The first 3 points of each leg (acquisition) are counted as normal points.
  - `notOnMotorway` (parallel road): ≥ 95 % of the points are `.notOnMotorway`.
  - `tooSlow`: ≥ 95 % `.tooSlow`.
  - Opposite direction (fixture 2): at no point is a facility returned by `AheadQuery` whose
    `strandId` is a strand other than the matched one, **and** every returned facility of kind
    `rastanlage` lies right of the matched strand (`d > 0` at its `lat/lon`).
- Datasets: the regional dataset `pipeline/out/regional/dataset.json` of this repository (not
  committed; the app's tests skip cleanly if missing) and the committed excerpt `fixtures/datasets/excerpt.json` (only the
  strands used by the fixtures plus their junctions and facilities, valid per schema).
