#!/usr/bin/env bash
# Python reference check for the Hoogle diff (P1).
#
# The oracle is deliberately a separate implementation: it must not import
# pvpdiff code. It runs only when the fetched real fixtures and the oracle
# script exist, so the default CI/sandbox path stays offline.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
oracle="$root/scripts/oracle/hoogle_oracle.py"

if [[ ! -f "$oracle" ]]; then
  echo "check-oracle: oracle not implemented yet; skipping (P1)"
  exit 0
fi

if [[ ! -d "$root/fixtures/real" ]]; then
  echo "check-oracle: fixtures/real is absent; run scripts/fetch-fixtures.sh first" >&2
  exit 0
fi

exec python3 "$oracle" "$root"
