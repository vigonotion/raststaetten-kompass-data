# Raststätten-Kompass dataset — project conventions

Rust CLI `rastpipe` that turns a Geofabrik OSM extract into the static dataset used by the iOS app
Raststätten-Kompass (app repository `vigonotion/raststaetten-kompass`, private). Public repository;
the weekly workflow publishes the dataset as a GitHub release.

Decisions and phase history live in the app repository's `PLAN.md` (references like `D-22`).

## Documents

1. `docs/schema.md` — dataset contract (FROZEN, schemaVersion 1). **Identical copy in the app
   repository**; change both together.
2. `docs/heuristics.md` — pipeline algorithms and parameters (§1–7, 9, 10, 11).
3. `docs/example-dataset.json` — reference input; identical copy in the app repository.

## Identifiers

| What | Value |
|---|---|
| Repo | `vigonotion/raststaetten-kompass-data` (public) |
| Rust crate / binary | `pipeline/` → `rastpipe` |
| Manifest URL (app default) | `https://github.com/vigonotion/raststaetten-kompass-data/releases/latest/download/manifest.json` |
| Release tag | `dataVersion` (OSM extract date, `YYYY-MM-DD`) |
| Dataset URL in the manifest | `https://github.com/vigonotion/raststaetten-kompass-data/releases/download/<dataVersion>/dataset.json.gz` |

## Languages

Code, identifiers, commit messages, comments, docs: English. Communication with the owner: German.

## Allowed dependencies

- **Rust:** `osmpbf`, `serde` (with `derive`), `serde_json`, `flate2`. Nothing else, also not as a
  dev-dependency. No `clap`, `tokio`, `anyhow`, `thiserror`, geo crates.
- **Scripts:** bash, `curl`, `jq`, `shasum`, `gzip`, standard Unix tools. No Python in committed scripts.
- Anything else: ask the owner first.

## Commands

```bash
# OSM extracts (→ data/osm/, gitignored). germany only after the owner's OK (the workflow does it in CI).
scripts/download-extract.sh hamburg|schleswig-holstein|niedersachsen|nordrhein-westfalen

# Pipeline (Rust via `. "$HOME/.cargo/env"`)
cd pipeline && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
cargo run --release -- build --input ../data/osm/hamburg-latest.osm.pbf \
  --brands ../data/brands.json --out out [--strict] [--no-autohoefe] [--dataset-url <url>]
cargo run --release -- validate --dataset ../docs/example-dataset.json [--brands ../data/brands.json]
# Regional reference build (replay tests and fixtures of the app repository)
target/release/rastpipe build --input ../data/osm/hamburg-latest.osm.pbf \
  --input ../data/osm/niedersachsen-latest.osm.pbf --input ../data/osm/nordrhein-westfalen-latest.osm.pbf \
  --brands ../data/brands.json --out out/regional --strict

# Release build with all checks (what .github/workflows/dataset.yml runs on the Germany extract)
scripts/release-dataset.sh --input <pbf> [--input <pbf>...] --out <dir>
```

If a tool is not found, retry in a login shell: `bash -l -c "…"`.

## Conventions

- **Commits:** Conventional Commits (`feat(pipeline): …`, `fix(pipeline): …`, `docs: …`, `ci: …`).
- **Parameters:** every numeric threshold from `docs/heuristics.md` is a named constant in
  `pipeline/src/params.rs`.
- **Geometry:** `docs/schema.md` §3 (R = 6 371 008.8 m, `d > 0` = right of the direction of travel).
- **Determinism:** same input → byte-identical `dataset.json`, `dataset.json.gz`, `manifest.json`.
- **Licenses:** code and `data/brands.json` MIT (`LICENSE`); released data ODbL 1.0
  (`LICENSE-DATA.md`). The dataset's `source` field carries "© OpenStreetMap contributors" and the
  ODbL URI; release notes repeat the attribution.

## Gates (every change)

- `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — all green.
- `Cargo.toml` dependencies only `osmpbf`, `serde`, `serde_json`, `flate2`.
- `rastpipe validate` passes on `docs/example-dataset.json` and the regional build (with
  `--brands ../data/brands.json`).
- Regional build with `--strict` succeeds (**0 invariant violations**).
- **Determinism:** build twice into two dirs, `cmp` `dataset.json`, `dataset.json.gz`,
  `manifest.json` → identical (`report.md` may differ only in timing/RSS lines).
- A change that alters behaviour must not silently change unrelated output: compare before/after
  numbers (strands, facilities by kind, unassigned, invariant, both-sides, brands).
- After changes that affect strands or facilities, the app repository's fixtures
  (`fixtures/gpx/`, `fixtures/datasets/excerpt.json`) must be regenerated there (see the READMEs
  in those directories) and its replay tests re-run.
- Before changing a heuristic, measure variants on the regional data and present them to the owner.
- `bash -n` on scripts; `actionlint` on workflows if available.

## Subagents

Definitions in `.claude/agents/` (`pipeline-dev`, `data-reviewer`), `model: sonnet`. They write only
in their area, add no dependencies, run the gates before reporting, never commit. The orchestrator
re-runs the gates itself after every report.
