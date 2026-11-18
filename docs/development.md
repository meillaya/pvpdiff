# Development

## Prerequisites

- Nix with flakes enabled (the repository is pinned to `nixpkgs`
  `d5cc6833999746cffdfe0598f7bc801a57b887a9`), and `devenv` 2.4+.
- `direnv` optional; `.envrc` simply runs `use devenv`.
- No system Rust is required; the shell pins `1.99.1` via
  `rust-toolchain.toml` and `devenv.nix`.

## Shells

```console
$ devenv shell          # or: direnv allow
$ devenv test           # fmt + clippy + tests (offline)
$ cargo test --workspace --all-features
$ cargo run -p pvpdiff-cli -- --version
```

`devenv test` runs `enterTest`, which is the same sequence as CI's fast job:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

The default Nix shell intentionally does **not** contain GHC/cabal. Local-mode
work (`pvpdiff check`) uses:

```console
$ nix develop .#local
$ ghc --version
$ cabal --version
```

The GHC shell is also where the optional local-mode integration test builds a
tiny fixture package and runs `cabal haddock --haddock-hoogle`.

## Flake outputs

| Output | Contents |
|---|---|
| `packages.default` / `packages.pvpdiff` | `rustPlatform.buildRustPackage` with `cargoLock.lockFile` |
| `devShells.default` | the devenv shell (Rust, test tooling) |
| `devShells.local` | GHC + cabal + cargo for local-mode checks |
| `checks.default` | build + test of the package |
| `apps.default` | `pvpdiff` |
| `formatter` | `nixfmt` |

`nix develop` must be called with `--impure`: the devenv module needs `PWD` to
set the project root, with a `/tmp` fallback for pure evaluation.

## Nix sandbox / read-only cache workaround

This machine's file sandbox occasionally denies writes to `~/.cache/nix`. Flake
evaluation can be checked with a redirected cache:

```console
$ export HOME=/tmp/nixhome
$ export XDG_CACHE_HOME=/tmp/nixhome/.cache
$ mkdir -p "$XDG_CACHE_HOME"
$ nix flake check --no-build --no-update-lock-file /path/to/pvpdiff
```

A full `nix build` needs a writable Nix store/cache; run it outside the sandbox
or request the needed permission. Redirecting `HOME` can break rustup/other
tools, so use it only for the Nix commands.

`devenv` also creates a per-user runtime directory under `/run/user/<uid>`. In
a read-only sandbox that fails with `Failed to create runtime directory`. Point
it at the repository instead:

```console
$ export XDG_RUNTIME_DIR="$PWD/.runtime"
$ mkdir -p "$XDG_RUNTIME_DIR"
$ chmod 700 "$XDG_RUNTIME_DIR"

$ nix develop --impure --command rustc --version   # 1.99.1
$ nix build .#default && ./result/bin/pvpdiff --version
$ nix shell github:NixOS/nixpkgs/d5cc6833999746cffdfe0598f7bc801a57b887a9#devenv -c devenv test
```

The generated `.runtime/`, `.cargo-home/`, and `result` entries are ignored by
git.

## Locks and toolchains

- `Cargo.lock`, `flake.lock`, and `devenv.lock` are committed.
- `flake.lock`/`devenv.lock` were copied from the working `hoard` scaffold
  because the input URLs/names are identical. If inputs change, regenerate with
  `nix flake lock`/`devenv update` and commit the result.
- Regenerate `Cargo.lock` inside the devenv shell (Rust 1.99.1) after changing
  dependencies; the workspace declares `rust-version = "1.99"` so the
  MSRV-aware resolver picks compatible crates. The pinned nixpkgs used by the
  flake also provides Rust 1.99.1.
- `CARGO_HOME` is set to `.cargo-home/` in the repo because the system
  `~/.cargo` is read-only here. Do not commit `.cargo-home/`.

## Build/test dependencies

- `reqwest` uses rustls + aws-lc-rs. The Nix package sets `AWS_LC_SYS_USE_SYSTEM`
  and provides `pkgs.aws-lc`; `pkg-config` is a native build input.
- `flate2` uses the pure-Rust `rust_backend` so the Nix build does not need
  system zlib.
- Network tests are `#[ignore]`/feature-gated. The default suite must run in the
  Nix sandbox without network or real Hackage fixtures.

## Fixtures

- `fixtures/synthetic/` is committed and covers parser edge cases.
- `fixtures/real/` is gitignored; `scripts/fetch-fixtures.sh` downloads the
  pinned package pairs and verifies SHA-256. Tests skip when real fixtures are
  absent.
- `fixtures/expected/` holds the curated expected classifications for the
  golden pairs; it is not raw Hackage data.

## Commit and changelog discipline

The project is built in small, green increments:

1. Before committing: `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
   `cargo test --workspace --all-features`; then `git status` to make sure no
   build artifacts, caches, or fetched fixtures are staged.
2. Subject: short imperative, <=72 characters, optional scope prefix
   (`parse:`, `core:`, `hackage:`, `cli:`, `p0:`, `ci:`, `docs:`). Body only for
   rationale or links.
3. Update `CHANGELOG.md` in the **same commit** as every user-visible change
   (command, flag, rule, schema, policy). Use the `[Unreleased]` section and
   Keep a Changelog categories. A commit that changes behaviour without a
   changelog entry is incomplete.
4. Phase gates are commits: scaffold, golden pairs, SARIF/Action, blast radius,
   survey dataset. Tag releases only when they are cut; move `Unreleased` to a
   version heading at that point.
5. Do not commit `fixtures/real/`, `data/`, `target/`, `result*`, caches, or
   generated release artifacts.

## Style

- Rust: edition 2024, `rustfmt`, clippy with `-D warnings`, `unsafe_code`
  denied, missing docs warned.
- Error types use `thiserror` in libraries and `anyhow` at the CLI boundary;
  never panic on untrusted input.
- Reports are deterministic: no timestamps, sorted collections, stable rule IDs,
  committed JSON Schemas.
- Documentation is written in the same plain, evidence-first style as the
  feasibility record; cite the PVP/Hackage source when a behaviour is not
  obvious.
