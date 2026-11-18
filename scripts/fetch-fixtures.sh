#!/usr/bin/env bash
# Fetch and verify the pinned Hackage fixtures used by the golden-pair tests.
#
# The P1 golden-pair work fills fixtures/pairs.tsv with the SHA-256 digests of
# each downloaded file. Until the manifest exists this script explains what is
# missing and exits 0, so a clean checkout and the Nix sandbox keep working.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$root/fixtures/pairs.tsv"
dest="$root/fixtures/real"

if [[ ! -f "$manifest" ]]; then
  echo "fetch-fixtures: no fixtures/pairs.tsv yet; skipping (P1 adds the pinned pairs)"
  exit 0
fi

mkdir -p "$dest"
base="${PVPDIFF_HACKAGE_URL:-https://hackage.haskell.org}"

while IFS=$'\t' read -r pkg from to from_docs_sha to_docs_sha from_cabal_sha to_cabal_sha; do
  [[ -z "${pkg:-}" || "$pkg" == \#* ]] && continue
  for ver in "$from" "$to"; do
    dir="$dest/$pkg/$ver"
    mkdir -p "$dir"
    curl -fsSL "$base/package/$pkg-$ver/docs/$pkg.txt" -o "$dir/$pkg.txt"
    curl -fsSL "$base/package/$pkg-$ver/$pkg.cabal" -o "$dir/$pkg.cabal"
  done
  # The manifest carries four digests per pair. Empty cells are a hard error:
  # never let an unverified fixture enter the test suite.
  if [[ -z "$from_docs_sha$to_docs_sha$from_cabal_sha$to_cabal_sha" ]]; then
    echo "fetch-fixtures: manifest rows must carry four SHA-256 digests (see fixtures/README.md)" >&2
    exit 1
  fi
  echo "$from_docs_sha  $dest/$pkg/$from/$pkg.txt" | sha256sum -c -
  echo "$to_docs_sha  $dest/$pkg/$to/$pkg.txt" | sha256sum -c -
  echo "$from_cabal_sha  $dest/$pkg/$from/$pkg.cabal" | sha256sum -c -
  echo "$to_cabal_sha  $dest/$pkg/$to/$pkg.cabal" | sha256sum -c -
done < "$manifest"

echo "fetch-fixtures: ok"
