#!/usr/bin/env bash
# Build, validate and verify the release artifacts of the dataset (PLAN D-07, D-51).
#
# Usage: scripts/release-dataset.sh --input <pbf> [--input <pbf>...] --out <dir>
#                                   [--repo vigonotion/raststaetten-kompass-data] [--brands data/brands.json]
#        scripts/release-dataset.sh --verify-only --out <dir> [--repo ...] [--brands ...]
#
# Runs `rastpipe build --strict` with the absolute release-asset URL as manifest url, validates the
# dataset, independently re-checks manifest.json against the files and writes release-notes.md.
# <dir> afterwards contains exactly: brands.json, dataset.json, dataset.json.gz, manifest.json,
# report.md, release-notes.md. brands.json is the brand list the dataset was built with (the app
# shows display names and logos from it). The last stdout line is the release tag (= dataVersion); if $GITHUB_OUTPUT is
# set, `tag=<dataVersion>` is appended to it. Any failed check exits non-zero.
#
# --verify-only skips the build and re-runs validation and manifest checks on an existing <dir>
# (writes release-notes.md; needs no --input, so the source file names come from dataset.json).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo="vigonotion/raststaetten-kompass-data"
brands="$REPO_ROOT/data/brands.json"
out=""
verify_only=0
inputs=()

usage() {
  echo "usage: $0 --input <pbf> [--input <pbf>...] --out <dir> [--repo <owner/name>] [--brands <json>]" >&2
  echo "       $0 --verify-only --out <dir> [--repo <owner/name>] [--brands <json>]" >&2
  exit 2
}
fail() { echo "error: $*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --input)  [[ $# -ge 2 ]] || usage; inputs+=("$2"); shift 2 ;;
    --out)    [[ $# -ge 2 ]] || usage; out="$2"; shift 2 ;;
    --repo)   [[ $# -ge 2 ]] || usage; repo="$2"; shift 2 ;;
    --brands) [[ $# -ge 2 ]] || usage; brands="$2"; shift 2 ;;
    --verify-only) verify_only=1; shift ;;
    *) usage ;;
  esac
done
[[ -n "$out" ]] || usage
if [[ $verify_only -eq 0 && ${#inputs[@]} -eq 0 ]]; then usage; fi
[[ "$repo" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] || fail "invalid --repo: $repo"
command -v jq >/dev/null || fail "jq not found"
command -v shasum >/dev/null || fail "shasum not found"

if ! command -v cargo >/dev/null 2>&1; then
  # shellcheck disable=SC1091
  [[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
fi

rastpipe="$REPO_ROOT/pipeline/target/release/rastpipe"
url_template="https://github.com/$repo/releases/download/{dataVersion}/dataset.json.gz"

if [[ $verify_only -eq 0 ]]; then
  for f in "${inputs[@]}"; do [[ -f "$f" ]] || fail "input not found: $f"; done
  [[ -f "$brands" ]] || fail "brands file not found: $brands"
  echo "Building rastpipe (release)"
  (cd "$REPO_ROOT/pipeline" && cargo build --release) >&2
  rm -rf "$out"
  mkdir -p "$out"
  args=()
  for f in "${inputs[@]}"; do args+=(--input "$f"); done
  echo "Building dataset"
  "$rastpipe" build "${args[@]}" --brands "$brands" --out "$out" --strict --dataset-url "$url_template" >&2
  cp "$brands" "$out/brands.json"
else
  [[ -d "$out" ]] || fail "output directory not found: $out"
  [[ -x "$rastpipe" ]] || (cd "$REPO_ROOT/pipeline" && cargo build --release) >&2
  [[ -f "$out/brands.json" ]] || cp "$brands" "$out/brands.json"
fi

for f in brands.json dataset.json dataset.json.gz manifest.json; do
  [[ -f "$out/$f" ]] || fail "missing $out/$f"
done

echo "Validating dataset"
"$rastpipe" validate --dataset "$out/dataset.json" --brands "$brands" >&2

echo "Verifying manifest"
file_size() { wc -c < "$1" | tr -d '[:space:]'; }
m="$out/manifest.json"
gz="$out/dataset.json.gz"
m_schema="$(jq -r '.schemaVersion' "$m")"
m_version="$(jq -r '.dataVersion' "$m")"
m_url="$(jq -r '.url' "$m")"
m_sha="$(jq -r '.sha256' "$m")"
m_size="$(jq -r '.sizeBytes' "$m")"
d_schema="$(jq -r '.schemaVersion' "$out/dataset.json")"
d_version="$(jq -r '.dataVersion' "$out/dataset.json")"
actual_sha="$(shasum -a 256 "$gz" | cut -d' ' -f1)"
actual_size="$(file_size "$gz")"
expected_url="${url_template//\{dataVersion\}/$d_version}"

[[ "$d_version" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || fail "dataVersion is not a date: $d_version"
[[ "$m_sha" == "$actual_sha" ]]       || fail "sha256 mismatch: manifest $m_sha, file $actual_sha"
[[ "$m_size" == "$actual_size" ]]     || fail "sizeBytes mismatch: manifest $m_size, file $actual_size"
[[ "$m_schema" == "$d_schema" ]]      || fail "schemaVersion mismatch: manifest $m_schema, dataset $d_schema"
[[ "$m_version" == "$d_version" ]]    || fail "dataVersion mismatch: manifest $m_version, dataset $d_version"
[[ "$m_url" == "$expected_url" ]]     || fail "url mismatch: manifest $m_url, expected $expected_url"
# The .gz must decompress to exactly dataset.json.
gzip -dc "$gz" | cmp -s - "$out/dataset.json" || fail "dataset.json.gz does not decompress to dataset.json"

# Source extracts: file names from --input, else from the dataset's `source` field.
if [[ ${#inputs[@]} -gt 0 ]]; then
  sources=""
  for f in "${inputs[@]}"; do sources+="- \`$(basename "$f")\`"$'\n'; done
else
  sources="- $(jq -r '.source' "$out/dataset.json")"$'\n'
fi

n_strands="$(jq '.strands | length' "$out/dataset.json")"
total_km="$(jq -r '[.strands[].lengthKm] | add | . * 10 | round / 10' "$out/dataset.json")"
n_rast="$(jq '[.facilities[] | select(.kind == "rastanlage")] | length' "$out/dataset.json")"
n_auto="$(jq '[.facilities[] | select(.kind == "autohof")] | length' "$out/dataset.json")"
n_junc="$(jq '.junctions | length' "$out/dataset.json")"

cat > "$out/release-notes.md" <<NOTES
# Raststätten-Kompass dataset $d_version

Offline dataset of German motorway service areas, generated from OpenStreetMap.

- **dataVersion / tag:** \`$d_version\`
- **schemaVersion:** $d_schema
- **SHA-256 (dataset.json.gz):** \`$actual_sha\`
- **Size (dataset.json.gz):** $actual_size bytes

## Source extracts

$sources
## Contents

| | |
|---|---|
| Motorway strands | $n_strands |
| Total length | $total_km km |
| Rastanlagen | $n_rast |
| Autohöfe | $n_auto |
| Junctions | $n_junc |

## License

The dataset files of this release (\`dataset.json.gz\`, \`manifest.json\`, \`report.md\`) contain
information from OpenStreetMap and are made available under the
[Open Database License (ODbL) 1.0](https://opendatacommons.org/licenses/odbl/1-0/)
(full text: [LICENSE-DATA.md](https://github.com/$repo/blob/main/LICENSE-DATA.md)).

Data © [OpenStreetMap contributors](https://www.openstreetmap.org/copyright).

If you use or redistribute this dataset, keep this attribution and offer any derived database under
the ODbL as well. \`brands.json\` and the code that builds the dataset are MIT-licensed
([LICENSE](https://github.com/$repo/blob/main/LICENSE)). Brand names are trademarks of their owners.
NOTES

# The release directory must hold exactly the five release files.
expected_files="brands.json
dataset.json
dataset.json.gz
manifest.json
release-notes.md
report.md"
actual_files="$(cd "$out" && ls -A | LC_ALL=C sort)"
[[ "$actual_files" == "$expected_files" ]] || fail "unexpected contents of $out: $(echo "$actual_files" | tr '\n' ' ')"

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "tag=$d_version" >> "$GITHUB_OUTPUT"
fi
echo "$d_version"
