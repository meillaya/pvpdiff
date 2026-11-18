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

### Known limitations

- Only the scaffold exists at this commit. The parser, diff, classification,
  SARIF output, local mode, reverse-dependency blast radius, and survey are
  delivered in phases P1-P4; see `docs/roadmap.md` once the documentation
  commit lands.
