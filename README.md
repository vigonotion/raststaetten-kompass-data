# Raststätten-Kompass dataset

Offline dataset of German motorway service areas (Rastanlagen) and truck stops (Autohöfe), built
from OpenStreetMap for the iOS app **Raststätten-Kompass**. The app downloads the newest release
from here; no location data is ever sent anywhere.

A new release is built every Monday at 03:00 UTC from the Geofabrik Germany extract. The release
tag is the date of the OSM data (`dataVersion`, e.g. `2026-10-19`).

## Download

The newest release always stays at the same address:

| File | Content |
|---|---|
| [`manifest.json`](https://github.com/vigonotion/raststaetten-kompass-data/releases/latest/download/manifest.json) | `schemaVersion`, `dataVersion`, absolute `url` of the dataset of the same release, `sha256`, `sizeBytes` |
| [`dataset.json.gz`](https://github.com/vigonotion/raststaetten-kompass-data/releases/latest/download/dataset.json.gz) | the dataset (gzip of the compact JSON) |
| [`report.md`](https://github.com/vigonotion/raststaetten-kompass-data/releases/latest/download/report.md) | build report: counts, invariant checks, samples for manual review |
| [`brands.json`](https://github.com/vigonotion/raststaetten-kompass-data/releases/latest/download/brands.json) | brand list the dataset was built with (ids, display names, categories, logo slugs) |

The format is described in [`docs/schema.md`](docs/schema.md) (schemaVersion 1), the algorithms
in [`docs/heuristics.md`](docs/heuristics.md).

## Build locally

Requirements: Rust (stable), `curl`, `jq`, `shasum`, `gzip`.

```bash
scripts/download-extract.sh hamburg          # → data/osm/ (germany needs --yes-germany, ~4.5 GB)
cd pipeline
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
cargo run --release -- build --input ../data/osm/hamburg-latest.osm.pbf \
  --brands ../data/brands.json --out out [--strict] [--no-autohoefe]
cargo run --release -- validate --dataset out/dataset.json --brands ../data/brands.json
```

A release build with all checks (what the weekly workflow runs):

```bash
scripts/release-dataset.sh --input data/osm/germany-latest.osm.pbf --out release
```

## License

- **Data** — the release files `dataset.json.gz`, `manifest.json` and `report.md` contain
  information from OpenStreetMap and are made available under the
  [Open Database License (ODbL) 1.0](https://opendatacommons.org/licenses/odbl/1-0/), see
  [`LICENSE-DATA.md`](LICENSE-DATA.md). Data © [OpenStreetMap contributors](https://www.openstreetmap.org/copyright).
  If you use the dataset, keep this attribution; if you publish a database derived from it, it
  must be offered under the ODbL as well.
- **Code and brand list** — everything else in this repository (pipeline, scripts, workflows,
  `data/brands.json`, docs) is under the [MIT License](LICENSE).
- Brand names are trademarks of their respective owners and are used only to identify the
  facilities.
