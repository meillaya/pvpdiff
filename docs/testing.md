# Testing

**Status:** P0 has the crate skeleton, CLI smoke tests, and the verification
commands. The suites below are built in P1–P4; this file is the plan of record.

## 1. Default suite (offline)

`devenv test` / `cargo test --workspace --all-features` must pass in the Nix
sandbox with no network and no real Hackage fixtures. Network access is opt-in.

| Suite | Location | Covers |
|---|---|---|
| Hoogle parser | `crates/pvpdiff-parse/tests` | header/`@package`/`@version`, module boundaries, all entry kinds, constructors/fields, GADTs, patterns, fixities, instances, class `where {}` blocks, docs/deprecation, adjacent entries, Unicode, no blank-line assumptions |
| Cabal adapter | `crates/pvpdiff-parse/tests` | synthetic multi-lib, `reexported-modules`, common stanzas, conditionals, old `Cabal-Version`, malformed/truncated → `Unknown` |
| Normalization | `crates/pvpdiff-core/tests` | alpha-renaming, context sorting, `*`/`Type`, synonyms, `$sel:`, pragmas, families → `Unknown`, era guard |
| Diff/rules | `crates/pvpdiff-core/tests` | full rule matrix, internal policy, `--strict`, re-export grouping, constructor aggregation, instance policy, version math (0.x, 2/4 components, wildcards), assessment, suggested versions |
| Golden pairs | `crates/pvpdiff-core/tests/golden.rs` | eight curated pairs (see §2); skips when the real fixtures are absent |
| CLI | `crates/pvpdiff-cli/tests` | text/JSON/Markdown, exit codes, stdout/stderr split, `--offline`, cache behavior |
| Cache/HTTP | `crates/pvpdiff-hackage/tests` | ETag/304, 404 docs, 429/Retry-After, redirects, truncated bodies, path traversal safety (`httpmock`) |
| SARIF | `crates/pvpdiff-core/tests/sarif.rs` | required GitHub fields, deterministic fingerprints, insta snapshot, roundtrip via `serde-sarif` |
| Index | `crates/pvpdiff-hackage/tests/index.rs` | synthetic `.tar.gz`, revisions last-entry-wins, unparseable Cabal, range `contains`, latest version, blast-radius categories |
| Schema drift | `crates/pvpdiff-core/tests/schema.rs` | committed `schemas/*.json` equals generated; `pvpdiff schema` equals committed |
| Property/robustness | parser/adapters | `proptest` arbitrary bytes/text never panics; reflexivity (same input → no changes) |
| Fuzz | `fuzz/` | Hoogle parser and Cabal adapter targets; a 1 h campaign before v0.1.0 |
| Changelog/commit | repository | phase-gate check: `git log` has incremental green commits; each command/rule/schema/policy change includes a `CHANGELOG.md` hunk |

## 2. Golden pairs

The curated set is the go/no-go evidence plus a false-breaking sensitivity set:

| # | Pair | Expected |
|---|---|---|
| 1 | `text-2.0.1 → 2.0.2` | additive; no breaking |
| 2 | `containers-0.6.7 → 0.7` | breaking |
| 3 | `aeson-2.1.2.1 → 2.2.0.0` | breaking |
| 4 | `bytestring-0.11.5.3 → 0.12.0.0` | breaking |
| 5 | `text-1.2.5.0 → 2.0` | breaking |
| 6 | `aeson-2.1.1.0 → 2.1.2.0` | additive/none; no breaking |
| 7 | `containers-0.6.6 → 0.6.7` | additive/none; no breaking |
| 8 | `lens-5.2.2 → 5.2.3` | additive/none; no breaking |

Only the curated expectations are committed (not raw Hackage data).
`scripts/fetch-fixtures.sh` downloads the eight pairs' docs/cabal with pinned
SHA-256 and verifies them; tests skip with a notice when `fixtures/real/` is
absent. CI fetches the fixtures before the test job. The expected files also
record the dominant rule IDs, not only the aggregate verdict.

The P1 gate is: pairs 1/6/7/8 show zero `breaking`; pairs 2–5 are classified
`breaking`; and the report's rule/entity details match the curated items. If the
normalizer cannot achieve that, stop and reassess.

## 3. Network/live tests

- `#[ignore]` tests fetch a pinned pair from Hackage and verify the embedded
  `@package`/`@version`, the SHA-256, and the cache revalidation path.
- A live job runs on `main`/manual dispatch only, never on PRs.
- No test asserts on undocumented Hackage HTML; the API surface is documented in
  [feasibility](feasibility.md).

## 4. Nix/CI

- `nix flake check` (evaluation) and `nix build .#default` (build + offline
  tests) must pass; `./result/bin/pvpdiff --version` is the smoke test.
- CI jobs: devenv test, cargo fmt/clippy/test/doc, Nix flake check/build,
  `cargo-deny`, actionlint/shellcheck.
- The local-mode GHC job is opt-in/main-only: it builds a tiny fixture package
  with `nix develop .#local`, runs `cabal haddock --haddock-hoogle`, and calls
  `pvpdiff check --docs`.

## 5. Acceptance criteria per phase

- **P0:** `devenv test` green; flake evaluates; package builds; feasibility doc
  and `CHANGELOG.md` exist; initial incremental commits.
- **P1:** eight pairs match; deterministic JSON; docs-missing → `unknown` +
  diagnostic; no panics; text/JSON/Markdown work; changelog current.
- **P2:** local `check` with `--docs` works; GHC fixture job passes; SARIF
  required-field tests pass; Action docs/actionlint; dist dry-run; changelog.
- **P3:** real index sync counts; blast radius reproduces approximate Hackage
  reverse counts; range evaluator tests; warm query target; changelog.
- **P4:** ≥5,000 packages classified; dataset/methodology/failure taxonomy
  published; outreach issues filed; release notes.
