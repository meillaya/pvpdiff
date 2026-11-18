# CLI contract

**Status:** P0 ships only `pvpdiff --version`/`--help`. The full surface below is
the P1–P4 target; commands appear incrementally and are added to
[CHANGELOG.md](../CHANGELOG.md) when they land.

## Commands

```console
pvpdiff diff --package <pkg> --from <ver> --to <ver>
             [--format text|json|markdown] [--json-out <file>] [--sarif <file>]
             [--sarif-uri <path>] [--strict] [--include-internal]
             [--fail-on never|breaking|unknown|any]
             [--cache-dir <dir>] [--offline] [--refresh]

pvpdiff diff --from-docs <old.txt> --to-docs <new.txt>
             [--from-cabal <old.cabal>] [--to-cabal <new.cabal>]

pvpdiff check --package-dir <dir> [--against latest|previous|<ver>]
              [--docs <file>] [--no-build] [--build-dir <dir>]

pvpdiff blast-radius --package <pkg> --new-version <ver>
                     [--stackage <snapshot>] [--json-out <file>]

pvpdiff index sync|status [--cache-dir <dir>]
pvpdiff survey run|report [--limit N] [--jobs N] [--shard N/M] [--out <file>]
pvpdiff schema [report|blast-radius|survey]
pvpdiff rules [--format text|json]
pvpdiff cache dir|clean
```

`--from`/`--to` also accept `latest` and `previous`. `--from` is the published
baseline, `--to` the candidate.

## Examples

```console
# Hackage-to-Hackage audit
$ pvpdiff diff --package text --from 2.0.1 --to 2.0.2

# machine-readable report + SARIF
$ pvpdiff diff --package aeson --from 2.1.2.1 --to 2.2.0.0 \
    --json-out pvpdiff.json --sarif pvpdiff.sarif

# local working tree against the latest published baseline
$ pvpdiff check --package-dir . --against latest --format markdown

# gate a release branch (opt-in; exit 100 on a violation candidate)
$ pvpdiff diff --package mypkg --from 1.2.3 --to 1.3.0 --fail-on breaking

# reverse-dependency bounds, after an explicit index sync
$ pvpdiff index sync
$ pvpdiff blast-radius --package text --new-version 2.1.4
```

## Output rules

- stdout carries the requested document (`text`, `json`, or `markdown`).
  Progress, warnings, and logs go to stderr, so
  `pvpdiff diff ... --format json | jq .` is safe.
- `--json-out` and `--sarif` write files; `--sarif-uri` sets the repository-
  relative URI embedded in SARIF locations (default `<package>.cabal`).
- Reports are deterministic: no timestamps, sorted arrays, stable rule IDs.
  `--format markdown` renders a changelog skeleton for the analysed package,
  separate from pvpdiff's own `CHANGELOG.md`.
- `--strict` counts exposed `Internal` modules and promotes instance removals,
  deprecations, fixity changes, and added class methods to `breaking`.
  `--include-internal` only widens internal-module counting.
- `--offline` uses the cache and fails with an actionable message if a required
  artifact is missing; `--refresh` bypasses immutable-doc caching.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | analysis completed, regardless of classification (advisory default) |
| `1` | tool, I/O, or network error (no report) |
| `100` | `--fail-on` gate triggered (`breaking`, `unknown`, or `any`) |
| `2` | Clap usage error (reserved by the argument parser) |

A `breaking`/`unknown` report is not an error by itself. CI fails only when the
caller asks for a gate. This matches the advisory positioning and
cargo-semver-checks' exit-code convention.

## Data and caching

- Hackage-mode inputs are the per-version `docs/<pkg>.txt` and `<pkg>.cabal`;
  both are cached with provenance digests in the report.
- `pvpdiff cache dir` prints the cache root; the layout is
  `<root>/hackage/<pkg>/<ver>/`.
- `pvpdiff schema` prints the committed JSON Schema; `pvpdiff rules` prints the
  stable rule catalog used by JSON and SARIF.
