# Roadmap

The phases are useful on their own; later phases do not require rewriting
earlier ones. Estimates assume part-time work.

| Phase | Scope | State |
|---|---|---|
| P0 | Repository scaffold: workspace, toolchain, devenv/flake, CI, docs skeleton, fixture pipeline, feasibility record | **Done** |
| P1 | Hackage diff MVP: parser, normalizer, PVP rules, version suggestion, changelog skeleton, Hackage client/cache, CLI `diff/schema/rules`, eight golden pairs | **Done** |
| P2 | SARIF, local `check` mode, composite GitHub Action, cargo-dist releases, GHC integration job | **Done** |
| P3 | Hackage index sync, reverse-dependency blast radius, Stackage filter | **Done** |
| P4 | All-Hackage survey, dataset + "PVP in the wild" report, advisory integration outreach | **In progress** |

## P0: Scaffold

- Four-crate workspace; Apache-2.0; pinned Rust 1.99.1; committed `Cargo.lock`.
- `devenv.nix`/`devenv.yaml` primary shell; `flake.nix` package, dev shells
  (including GHC for local mode), checks, apps, formatter; copied pinned locks.
- CI: devenv test, cargo fmt/clippy/test/doc, Nix flake check/build, cargo-deny,
  actionlint/shellcheck.
- Documentation: feasibility record (research evidence), design, PVP rules,
  CLI/report contracts, limits, development, integration, roadmap.
- `CHANGELOG.md` with `[Unreleased]`; commit-as-you-go policy.
- Fixture pipeline skeleton; synthetic fixture layout.

Exit: `devenv test` green, flake evaluates, package builds, docs and changelog
committed.

## P1: Hackage diff MVP

- `pvpdiff-parse`: Hoogle line-state parser (all entity kinds, constructors/
  fields, patterns, fixities, instances, class blocks, docs/deprecation) and the
  Cabal adapter.
- `pvpdiff-core`: normalization stages, diff, rule catalog, grouping, version
  math, assessment, suggested version, changelog skeleton, JSON/Markdown/text.
- `pvpdiff-hackage`: URLs, blocking client, ETag/content cache, provenance.
- `pvpdiff-cli`: `diff`, `schema`, `rules`, `cache`.
- Eight golden pairs + synthetic fixtures + property/fuzz tests.

Exit: P1 gate in [testing](testing.md) §2; deterministic reports; changelog per
rule/command.

## P2: CI workflow

- SARIF 2.1.0 (`serde-sarif`) with deterministic fingerprints and locations.
- `pvpdiff check --package-dir . --against latest`; `cabal haddock
  --haddock-hoogle` discovery; `--docs` bypass.
- Composite Action (`action.yml`) downloading release binaries; example
  workflow with upload-sarif@v4; cargo-dist config and generated release
  workflow.
- GHC/local-mode integration test.

Exit: `nix develop .#local` fixture package works; SARIF tests pass; actionlint;
`v0.1.0-rc` dry-run.

## P3: Blast radius

- `pvpdiff index sync|status`: conditional GET `01-index.tar.gz`, streaming
  parse, last-revision-wins, JSONL dependency database + metadata.
- `pvpdiff blast-radius`: Cabal version-range evaluation, excluded/compatible/
  no-upper-bound/over-constrained categories.
- Stackage `cabal.config` join as an optional filter.

Exit: real index sync counts; text example sanity-checked against Hackage's
reverse page; warm query target.

## P4: Dataset and integration

- `pvpdiff survey run|report`: resumable JSONL, jobs/shards, rate limiting.
- Full Hackage run over the ~73% with both docs; publish the dataset, coverage,
  violation-candidate rates, top rules, and failure taxonomy.
- Advisory integration: GitHub Action docs/demo; pvp.haskell.org tools entry
  after a track record; comment on cabal PR #12363; Hackage advisory discussion
  (non-gating).

Exit: ≥5,000 packages classified; dataset published; outreach filed.

## Stop points

- End of P1: a useful standalone CLI.
- End of P2: the Action/demo and the main adoption wedge.
- P3/P4 are the ecosystem-reputation layer, not prerequisites for the MVP.

If the P1 gate fails, stop and reassess the normalization promise rather than
adding heuristics that hide uncertainty.
