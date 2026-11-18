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

- Only the scaffold exists at this commit. The parser, diff, classification,
  SARIF output, local mode, reverse-dependency blast radius, and survey are
  delivered in phases P1-P4; see `docs/roadmap.md`.
