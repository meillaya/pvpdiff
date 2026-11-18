# CLI contract

Status: `diff`, `check`, `index`, `blast-radius`, `survey`, `schema`, `rules`,
and `cache` are implemented. Commands are added to
[CHANGELOG.md](../CHANGELOG.md) when they land.

## Commands

```console
pvpdiff diff --package <pkg> --from <ver> --to <ver>
             [--format text|json|markdown] [--json-out <file>] [--sarif <file>]
             [--sarif-uri <path>] [--sarif-all] [--strict] [--include-internal]
             [--fail-on never|breaking|unknown|any]
             [--cache-dir <dir>] [--offline] [--refresh]

pvpdiff diff --from-docs <old.txt> --to-docs <new.txt>
             [--from-cabal <old.cabal>] [--to-cabal <new.cabal>]

pvpdiff check --package-dir <dir> [--against latest|previous|<ver>]
              [--docs <file>] [--no-build] [--build-dir <dir>]
              [--format text|json|markdown] [--json-out <file>] [--sarif <file>]
              [--fail-on never|breaking|unknown|any]

pvpdiff blast-radius --package <pkg> --new-version <ver>
                     [--stackage <snapshot>] [--json-out <file>]
                     [--format text|json] [--limit <rows>]
                     [--cache-dir <dir>] [--offline] [--refresh]

pvpdiff index sync|status [--cache-dir <dir>] [--offline] [--refresh]
                          [--index-url <url>]
pvpdiff survey run [--out <file>] [--limit N] [--jobs N] [--max-versions N]
                   [--delay-ms N] [--shard I/N] [--cache-dir <dir>]
                   [--offline] [--refresh]
pvpdiff survey report [--input <file>] [--out <file>]
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

# local working tree plus SARIF
$ pvpdiff check --package-dir . --against latest --sarif pvpdiff.sarif

# gate a release branch (opt-in; exit 100 on a violation candidate)
$ pvpdiff diff --package mypkg --from 1.2.3 --to 1.3.0 --fail-on breaking

# reverse-dependency bounds, after an explicit index sync
$ pvpdiff index sync --cache-dir ~/.cache/pvpdiff
$ pvpdiff blast-radius --package text --new-version 2.1.4 \
    --cache-dir ~/.cache/pvpdiff --limit 20

# filter the report to a Stackage snapshot
$ pvpdiff blast-radius --package text --new-version 2.1.4 \
    --stackage lts-24.67

# resumable survey; the default output is data/survey-v1.jsonl
$ pvpdiff survey run --limit 5000 --jobs 4 --delay-ms 200
$ pvpdiff survey report --input data/survey-v1.jsonl
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
- `pvpdiff schema [report|blast-radius]` prints the committed JSON Schema;
  `pvpdiff rules` prints the stable rule catalog used by JSON and SARIF.
- `pvpdiff index sync` is a conditional GET of the 141 MB `01-index.tar.gz`.
  It writes a JSONL dependency database and metadata under `<cache>/index/`.
  A warm run uses the ETag and returns `304` without reparsing.
- `pvpdiff survey run` is resumable: records already present for a package pair
  are skipped. A skipped record is terminal on later runs; remove it or use a
  fresh output file to retry a transient fetch failure.
- `pvpdiff survey report` is deterministic. JSONL record order can vary with
  worker scheduling; the aggregate report sorts its input.
