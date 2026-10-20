#!/usr/bin/env bash
# Download a Geofabrik OSM extract into data/osm/ and verify its MD5 checksum.
#
# Usage: scripts/download-extract.sh <region> [--yes-germany]
#   region: hamburg | schleswig-holstein | niedersachsen | nordrhein-westfalen | germany
#
# The whole-Germany extract (~4-5 GB) requires the explicit flag --yes-germany.
# Re-running is cheap: an existing file with a matching checksum is not downloaded again.
set -euo pipefail

BASE_URL="https://download.geofabrik.de/europe"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${OSM_DIR:-$REPO_ROOT/data/osm}"

usage() {
  echo "usage: $0 <hamburg|schleswig-holstein|niedersachsen|nordrhein-westfalen|germany> [--yes-germany]" >&2
  exit 2
}

[[ $# -ge 1 ]] || usage
region="$1"
allow_germany="${2:-}"

case "$region" in
  hamburg|schleswig-holstein|niedersachsen|nordrhein-westfalen)
    url="$BASE_URL/germany/$region-latest.osm.pbf" ;;
  germany)
    if [[ "$allow_germany" != "--yes-germany" ]]; then
      echo "error: the germany extract is ~4-5 GB; pass --yes-germany to confirm." >&2
      exit 2
    fi
    url="$BASE_URL/germany-latest.osm.pbf" ;;
  *) usage ;;
esac

mkdir -p "$OUT_DIR"
file="$OUT_DIR/$region-latest.osm.pbf"
md5_file="$file.md5"

md5_of() {
  if command -v md5sum >/dev/null 2>&1; then md5sum "$1" | cut -d' ' -f1
  else md5 -q "$1"; fi
}

# Geofabrik redirects `<region>-latest.osm.pbf` to mirrors that can lag a day behind, and for
# `germany` no `-latest.osm.pbf.md5` exists any more. Resolve the newest dated file
# (`<region>-YYMMDD.osm.pbf`) from the region's index page and download it together with its own
# checksum from download.geofabrik.de, so file and checksum always belong together.
index_url="${url%-latest.osm.pbf}.html"
dated="$(curl -fsSL "$index_url" | grep -oE "href=\"$region-[0-9]{6}\.osm\.pbf\"" \
  | sed -E 's/^href="(.*)"$/\1/' | LC_ALL=C sort -u | tail -n 1 || true)"
if [[ -n "$dated" ]]; then
  url="${url%/*}/$dated"
else
  echo "warning: no dated extract found on $index_url; using $url" >&2
fi

echo "Fetching checksum: $url.md5"
curl -fsSL "$url.md5" -o "$md5_file"
expected="$(cut -d' ' -f1 "$md5_file")"

if [[ -f "$file" && "$(md5_of "$file")" == "$expected" ]]; then
  echo "Up to date: $file"
  exit 0
fi

echo "Downloading: $url"
curl -fL --retry 3 --retry-delay 5 -C - "$url" -o "$file.part" || {
  # A stale .part from an older extract cannot be resumed; start over once.
  rm -f "$file.part"
  curl -fL --retry 3 --retry-delay 5 "$url" -o "$file.part"
}

actual="$(md5_of "$file.part")"
if [[ "$actual" != "$expected" ]]; then
  rm -f "$file.part"
  echo "error: checksum mismatch for $region (expected $expected, got $actual)" >&2
  exit 1
fi
mv "$file.part" "$file"
echo "OK: $file ($(du -h "$file" | cut -f1))"
