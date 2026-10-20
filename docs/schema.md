# Dataset Contract — schemaVersion 1 (FROZEN)

Binding for the Rust pipeline (writer, repository `vigonotion/raststaetten-kompass-data`) and
RastCore (reader, app repository). This file exists **identically in both repositories**; a change
is made in both at once, with an entry in the app's `PLAN.md` → "Schema changes". Subagents that
need a change stop and report back with a concrete proposal.

Reference example: `docs/example-dataset.json` (must always validate against this document;
both `rastpipe validate` and the RastCore tests use it).

## 1. Files

| File | Producer | Content |
|---|---|---|
| `dataset.json` | `rastpipe build` | Dataset (§2), UTF-8, no BOM, compact (no pretty-printing) |
| `dataset.json.gz` | `rastpipe build` | gzip (RFC 1952) of exactly the bytes of `dataset.json` |
| `manifest.json` | `rastpipe build` / `scripts/release-dataset.sh` | Manifest (§4) |
| `report.md` | `rastpipe build` | Validation report (not part of the contract, format in `docs/heuristics.md` §9) |

## 2. Dataset

```json
{
  "schemaVersion": 1,
  "dataVersion": "2026-10-19",
  "generatedAt": "2026-10-19T03:00:00Z",
  "source": "Geofabrik germany-latest.osm.pbf, © OpenStreetMap contributors, ODbL 1.0 (https://opendatacommons.org/licenses/odbl/1-0/)",
  "strands":    [ Strand, ... ],
  "junctions":  [ Junction, ... ],
  "facilities": [ Facility, ... ]
}
```

| Field | Type | Rules |
|---|---|---|
| `schemaVersion` | int | Always `1` for this contract. |
| `dataVersion` | string | `YYYY-MM-DD` = UTC date of the OSM extract (Geofabrik timestamp; fallback: build date). Compared **lexicographically**. |
| `generatedAt` | string | RFC 3339 UTC with `Z`, second precision. |
| `source` | string | Human-readable source + attribution. Must contain `© OpenStreetMap contributors` and `ODbL`. With several inputs: comma-separated file names. |
| `strands` | array | Sorted by `id` (byte order). |
| `junctions` | array | Sorted by `strandId`, then `km`. |
| `facilities` | array | Sorted by `strandId`, then `km`, then `id`. |

### 2.1 Strand

One directed carriageway chain of one motorway number.

```json
{ "id": "A1-SW-0", "ref": "A 1", "towards": "SW", "polyline": "…", "lengthKm": 742.312 }
```

| Field | Type | Rules |
|---|---|---|
| `id` | string | `<ref without spaces>-<towards>-<n>`, e.g. `A1-SW-0`, `A7-N-2`. `n` is 0-based per (`ref`, `towards`), assigned by descending `lengthKm` (ties: smaller first OSM way ID of the chain). Matches `^A[0-9]+-(N\|NO\|O\|SO\|S\|SW\|W\|NW)-[0-9]+$`. |
| `ref` | string | Normalized `A <number>` with exactly one space (`A1`, `A  1`, `a 1` → `A 1`). Only A numbers; letter suffixes are not used on German motorways. |
| `towards` | string | One of `N, NO, O, SO, S, SW, W, NW` (German abbreviations). Initial bearing (great circle) from the first to the last point, 8 sectors of 45° centered on the direction: N = [337.5°, 22.5°), NO = [22.5°, 67.5°), … |
| `polyline` | string | Google Encoded Polyline, precision 1e5, order **lat, lon**, in direction of travel. At least 2 points. |
| `lengthKm` | number | Sum of the Haversine distances of the **decoded** (rounded) points, in km, rounded to 3 decimal places. |

Direction of travel = order of the points. Every point of a strand has a km position:
`km(point i)` = cumulative Haversine distance from point 0 along the decoded points.

### 2.2 Junction

A `highway=motorway_junction` node on the strand (exit or interchange).

```json
{ "strandId": "A1-SW-0", "km": 12.105, "ref": "38", "name": "Hamburg-Harburg" }
```

| Field | Type | Rules |
|---|---|---|
| `strandId` | string | Existing `Strand.id`. |
| `km` | number | Projection onto the strand (§3), 3 decimal places, `0 ≤ km ≤ lengthKm`. |
| `ref` | string \| null | OSM `ref` of the junction (e.g. `"38"`, `"12a"`), trimmed; otherwise `null`. |
| `name` | string \| null | OSM `name`, trimmed; otherwise `null`. |

At least one of `ref`/`name` is set; junctions without either are not emitted.

### 2.3 Facility

```json
{
  "id": "osm-way-123456",
  "kind": "rastanlage",
  "name": "Raststätte Stillhorn West",
  "strandId": "A1-SW-0",
  "km": 14.402,
  "lat": 53.48761, "lon": 10.01847,
  "exit": null,
  "detourKm": 0,
  "brands": ["burger-king", "aral"],
  "shower": true,
  "fastCharger": false,
  "open24h": true
}
```

| Field | Type | Rules |
|---|---|---|
| `id` | string | Unique within the dataset. Base ID `osm-<type>-<osmId>` with `type` ∈ `node`, `way`, `relation`. Rastanlage: the base ID; if the facility is emitted on **several** strands (shared section, e.g. `A 1;A 3`, `docs/heuristics.md` §4.4), **every** entry gets the suffix `@<strandId>`. Autohof: always `osm-<type>-<osmId>@<strandId>` (one entry per direction of travel; `type`/`osmId` of the services object if it is one, else of the fuel station — D-31). Readers derive the OSM object from the part before `@`. |
| `kind` | string | `rastanlage` \| `autohof`. |
| `name` | string \| null | OSM `name` of the facility (Autohof: of the services object if it is one, else of the fuel station — D-31), trimmed. `null` if missing; the app then shows a fallback text. |
| `strandId` | string | Existing `Strand.id`. |
| `km` | number | Rastanlage: projection of the centroid onto the strand. Autohof: `km` of the junction on this strand. 3 decimal places. `0 ≤ km ≤ lengthKm`. |
| `lat`, `lon` | number | WGS84, 5 decimal places. Rastanlage: centroid (§3). Autohof: position of the fuel station (merged into a services object: still the fuel station; services object without fuel station: its centroid). Used as the target for Apple Maps. |
| `exit` | string \| null | Rastanlage: always `null`. Autohof: display text of the junction: `"<ref> <name>"` if both exist, otherwise whichever exists, e.g. `"38 Hamburg-Harburg"`. OSM lists (`;`) are paired value by value and joined with `" / "`: ref `24;23` + name `Dreieck Delmenhorst;Delmenhorst-Hasport` → `"24 Dreieck Delmenhorst / 23 Delmenhorst-Hasport"`; unequal list lengths → refs and names each joined with `" / "`. The same normalization applies to `Junction.ref`/`name`. |
| `detourKm` | number | Rastanlage: `0`. Autohof: straight-line distance junction node → fuel station × 1.3, in km, 1 decimal place. |
| `brands` | string[] | Canonical IDs from `data/brands.json`, deduplicated, **ordered as in `brands.json`**. May be empty. |
| `shower` | bool | See `docs/heuristics.md` §6. |
| `fastCharger` | bool | See `docs/heuristics.md` §6. |
| `open24h` | bool | See `docs/heuristics.md` §6. |

## 3. Geometry conventions (both sides)

- Earth radius: **R = 6 371 008.8 m** (mean radius). Haversine everywhere distances are computed.
- Bearing: initial great-circle bearing in degrees `[0, 360)`, 0 = north, clockwise.
- Local metric computations (projection, lateral offset, point in polygon): equirectangular
  projection around the latitude of the reference point:
  `x = R·Δλ·cos(φ0)`, `y = R·Δφ` (radians).
- **Signed lateral offset `d`:** for segment `A→B` and point `P` in local coordinates:
  `cross = (B−A) × (P−A) = (Bx−Ax)(Py−Ay) − (By−Ay)(Px−Ax)`; `d = −cross / |B−A|`.
  **`d > 0` means right of the direction of travel**, `d < 0` left.
- Projection of a point onto a strand: nearest point over all segments (clamped to the segment);
  `km` = km of the segment start point + Haversine distance from segment start to the foot point.
- Rounding: only on output (km/lengthKm 3 decimal places, lat/lon 5, detourKm 1). All
  computations use unrounded values; `lengthKm` and strand `km` refer to the decoded points.

## 4. Manifest

```json
{
  "schemaVersion": 1,
  "dataVersion": "2026-10-19",
  "url": "https://github.com/vigonotion/raststaetten-kompass-data/releases/download/2026-10-19/dataset.json.gz",
  "sha256": "3a7bd3e2360a3d29eea436fcfb7e44c735d117c42d1c1835420b6b9942dd4f1b",
  "sizeBytes": 1234567
}
```

| Field | Type | Rules |
|---|---|---|
| `schemaVersion` | int | Equal to the dataset's `schemaVersion`. |
| `dataVersion` | string | Equal to the dataset's `dataVersion`. |
| `url` | string | Absolute URL **or** relative path, resolved against the manifest URL (RFC 3986). Local build: `"dataset.json.gz"`. Release: absolute URL to the **dated tag** (not `latest`), so manifest and file always belong together. |
| `sha256` | string | SHA-256 of the bytes of `dataset.json.gz`, 64 lowercase hex characters. |
| `sizeBytes` | int | Size of `dataset.json.gz` in bytes. |

## 5. Reader rules (RastCore)

- Accepted are only datasets and manifests with `schemaVersion == 1`. Anything else is discarded
  (no error to the user; fallback to the last valid dataset).
- Update is loaded when `manifest.dataVersion > current.dataVersion` (lexicographic) and
  `schemaVersion` is known. After the download: check `sizeBytes` and `sha256` against the
  manifest, unpack gzip, decode, check `schemaVersion`/`dataVersion` against the manifest.
  Any error → discard, keep the old dataset.
- Unknown JSON fields are ignored (decoders must not fail on them). Missing required fields → dataset invalid.
- Referential integrity (`strandId` exists) is checked by `rastpipe validate`; the reader
  skips facilities/junctions with an unknown `strandId`.

## 6. `data/brands.json`

```json
[
  {
    "id": "burger-king",
    "displayName": "Burger King",
    "category": "food",
    "match": { "brand": ["Burger King"], "name": ["Burger King"] },
    "simpleIconsSlug": "burgerking"
  }
]
```

| Field | Type | Rules |
|---|---|---|
| `id` | string | kebab-case `^[a-z0-9]+(-[a-z0-9]+)*$`, unique. Stable: never rename (it is persisted in filters). |
| `displayName` | string | Display name in the app. |
| `category` | string | `food` \| `coffee` \| `fuel` \| `charging` \| `shop` \| `other`. |
| `match.brand` | string[] | Values for the OSM `brand` tag (matching rules: `docs/heuristics.md` §5). |
| `match.name` | string[] | Patterns for the OSM `name` tag (whole word, see heuristics). May be empty. |
| `simpleIconsSlug` | string \| null | Slug verified against the pinned Simple Icons version, otherwise `null`. |

The order in the file is the curated order (categories grouped). It determines the order of
`Facility.brands`. "Most frequent first" in the app filter is computed from the dataset at runtime.
