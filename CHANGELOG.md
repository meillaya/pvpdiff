# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for its own releases. (The tool analyses Haskell packages that follow the PVP;
pvpdiff's own version number does not.)

## [Unreleased]

### Added

- Four-crate Rust workspace (`pvpdiff-parse`, `pvpdiff-core`,
  `pvpdiff-hackage`, `pvpdiff-cli`) on edition 2024, Apache-2.0, with a pinned
  1.99.1 toolchain and a committed `Cargo.lock`.
- `devenv.nix` development shell (Rust, Python for fixture/oracle scripts,
  `jq`, `cargo-deny`, `cargo-insta`, `actionlint`, `shellcheck`) and a
  `flake.nix` exposing the package, dev shells, checks, apps, and formatter,
  pinned to the same nixpkgs/devenv/rust-overlay revisions as the working
  `hoard` scaffold. A separate `devShells.local` carries GHC and cabal for
  local-mode checks.
- CLI skeleton: `pvpdiff --version` and `--help`; no analysis commands yet.
- Workspace lints deny `unsafe_code` and unsafe operations in unsafe functions;
  missing docs and missing `Debug` implementations are warnings. The CLI has
  smoke tests for `--version` and `--help`.
- CI workflows: devenv test, cargo format/clippy/test/doc, Nix flake
  check/build, `cargo-deny`, and actionlint plus shellcheck.
- Fixture pipeline skeleton: `scripts/fetch-fixtures.sh` (SHA-256-pinned
  downloads; refuses unverified fixtures), `scripts/check-oracle.sh`, synthetic
  and expected-fixture layout, and an offline-safe default test posture.
- `docs/feasibility.md`: the research record for the Hoogle-based PVP diff,
  including the five-pair go/no-go, coverage measurements, derivability limits,
  prior-art survey, and data-source matrix.
- `docs/pvp.md`: the rule catalog and the explicit PVP interpretation used by
  the classifier.
- Repository documentation: design, CLI and report contracts, testing,
  limitations, roadmap, integration, and development guides, plus the README.
- `examples/github-workflow.yml`: the planned advisory Action workflow (P2).

### Added

- `pvpdiff diff` for Hackage-to-Hackage and local Hoogle database pairs.
  It reports a tri-state classification, confidence, observed PVP bump,
  assessment, suggested next version, per-class counts, and a grouped change
  list in text, JSON, or Markdown.
- Hoogle text parser: module sections, functions, data/newtype heads,
  constructors, record fields, GADTs, pattern synonyms, type synonyms, type and
  data families, classes with `where {}` blocks, fixities, instances,
  deprecation flags, and embedded `@package`/`@version` validation.
- Cabal adapter over `cabalist-parser` with `reexported-modules`, common-stanza
  resolution, conditional surface detection, and library dependency extraction.
- Normalizer with whitespace and pragma cleanup, `*`/`Type` handling,
  alpha-renaming, context sorting, synonym/family uncertainty, primitive-type
  and hole guards, qualification removal for instances, and a renderer-era
  guard.
- PVP rule catalog (`pvpdiff rules`), conservative defaults, `--strict`,
  `--include-internal`, and `--fail-on` with exit code 100.
- Hackage client with a content cache, ETag and Last-Modified revalidation,
  offline mode, SHA-256 provenance, and documented redirect handling.
- `pvpdiff schema` prints the committed report schema
  (`schemas/pvpdiff-v1.schema.json`); `pvpdiff cache dir|clean` manages the
  local cache.

- `pvpdiff check`: local working-tree mode. It reads the package `.cabal`,
  builds `cabal haddock --haddock-hoogle` under a cache build directory (or
  accepts `--docs`), selects a Hackage baseline, and compares the two. `--no-build`
  requires `--docs`; the multi-library limitation is reported.
- SARIF 2.1.0 output with stable rule metadata, error/warning/note levels,
  artifact locations, and deterministic fingerprints. Flags: `--sarif`,
  `--sarif-uri`, `--sarif-all`.
- The eight golden pairs are pinned by SHA-256 in `fixtures/pairs.tsv`; the
  fetcher verifies every file and the curated expectations live in
  `fixtures/expected/`. A golden test checks classification, assessment,
  suggested version, counts, required/forbidden rules, and named changes.
- Schema drift test for `schemas/pvpdiff-v1.schema.json`; report unavailability
  now produces an `unknown` report with a diagnostic and exit code 0.
- Composite GitHub Action (`action.yml`), workflow documentation, and
  `dist-workspace.toml` with the generated cargo-dist release workflow.
- CI fetches the pinned fixtures before tests and adds a local-mode job that
  builds docs with GHC 9.10.3 and runs `pvpdiff check` plus SARIF assertions.

- `pvpdiff index sync|status`: conditional GET of Hackage's `01-index.tar.gz`
  streamed into a JSONL dependency database with revision last-wins semantics,
  SHA-256 provenance, ETag reuse, offline reads, and atomic writes.
- `pvpdiff blast-radius`: selects the newest indexed version of every dependent
  and classifies its target bounds as `excluded`, `compatible`,
  `no-upper-bound`, or `over-constrained`. Text and JSON output; optional
  Stackage `cabal.config` pin filter; `pvpdiff schema blast-radius` prints the
  committed schema.
- Synthesized test suite for index sync, range evaluation, status rules, and
  snapshot pins, including synthetic tar archives with duplicate revisions.

### Fixed

- Lowercase functions that return their enclosing data type are no longer
  mistaken for constructors.
- Orphan-instance detection now reads the type head after the class token.
- Writing reports to a closed stdout pipe exits cleanly instead of panicking.
- Reverse-dependency selection now chooses the newest version of each package
  before filtering on the target dependency, so a package that dropped the
  dependency is not counted through an older release.

### Changed

- The initial project plan narrows the promise from "PVP compliance" to
  "breaking-change candidates plus bump advice", because the Hoogle text
  database cannot prove semantic compatibility. This is recorded in the
  feasibility document, limitations, and CLI wording.
- P0 is verified on this machine: `nix flake check` evaluates every output,
  `nix build .#default` produces a working `pvpdiff 0.1.0` binary, and
  `devenv test` passes. The development guide records the read-only-sandbox
  workaround (`XDG_RUNTIME_DIR` and a redirected Nix cache); the roadmap marks
  P0 done.

### Known limitations

- The survey and advisory integration are the remaining P4 work; see
  `docs/roadmap.md`. Blast-radius counts are index-based approximations, not
  Hackage's page counts.
