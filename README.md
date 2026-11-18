# pvpdiff

Compare two versions of a Haskell package from Hackage's per-version Hoogle
databases. The tool runs without GHC or `.hie` files, and returns a classified
API diff, a PVP bump assessment, a suggested next version, a changelog skeleton,
and SARIF for CI.

> Status: P4. Hackage and local diffs, SARIF, the Action, index sync, blast
> radius, and the resumable Hackage survey are implemented. The
> [PVP in the wild](docs/pvp-in-the-wild.md) dataset report is committed. See
> [roadmap](docs/roadmap.md).

> Scope: `pvpdiff` reports breaking-change candidates, not proof of PVP
> compliance. Hoogle data cannot see behaviour-only changes, roles, family
> equations, default method bodies, or instances for external types. The tool
> uses a conservative `unknown` bucket and never fails CI by default. Read
> [limitations](docs/limitations.md) before relying on it.

## Quick start

```console
$ pvpdiff diff --package text --from 2.0.1 --to 2.0.2
pvpdiff 0.1.0
text 2.0.1 -> 2.0.2

Classification: additive (high confidence)
PVP: observed minor bump; assessment: consistent
Suggested next version: 2.0.2

ADDITIVE (19)
  + Data.Text.Encoding.decodeASCIIPrefix  ByteString -> (Text, ByteString)
  ...
```

```console
$ pvpdiff diff --package aeson --from 2.1.2.1 --to 2.2.0.0 \
    --json-out pvpdiff.json --sarif pvpdiff.sarif

$ pvpdiff check --package-dir . --against latest
```

```console
$ pvpdiff index sync
$ pvpdiff blast-radius --package text --new-version 2.1.4

$ pvpdiff survey run --limit 10000 --jobs 12 --delay-ms 75
$ pvpdiff survey report --input data/survey-v1.jsonl
```

The committed [PVP in the wild](docs/pvp-in-the-wild.md) report covers 5,940
analyzed package pairs from a 10,000-package slice.

Other commands:

- `pvpdiff check --package-dir . --against latest`: compare the local working
  tree with the published Hackage baseline, using
  `cabal haddock --haddock-hoogle`.
- `pvpdiff blast-radius --package text --new-version 2.1.4`: list Hackage
  dependents whose bounds exclude the new version, from the cached Hackage
  index.
- `pvpdiff survey`: a reproducible "PVP in the wild" dataset over Hackage.

## Why this is not another API-diff tool

API diffing itself has prior art: [`hackage-diff`](https://github.com/blitzcode/hackage-diff)
(dead since 2018), [`packdiff`](https://github.com/composewell/packdiff)
(active, Streamly CI), [`print-api`](https://github.com/Kleidukos/print-api),
and a proposed informational packdiff job in
[cabal#12363](https://github.com/haskell/cabal/pull/12363). What does not exist
is the end-to-end PVP workflow: a stable rule catalog, PVP bump assessment
and suggested version, a changelog skeleton, SARIF/Action integration, and
reverse-dependency blast radius. That is the gap this project targets.

The design is grounded in a five-pair go/no-go against real changelogs; see
[`docs/feasibility.md`](docs/feasibility.md). Key measured facts:

- after modest normalization, the five pairs matched the changelog severity
  5/5 (the naive patch-pair diff would have falsely accused a legal patch
  release);
- both latest and previous Hoogle databases exist for 54/74 (73%) of random
  packages, and 90/100 for LTS-pinned versions. Hackage-only analysis is
  therefore useful but limited; local mode covers unpublished code;
- exposed `*.Internal` modules and doc-build GHC instance churn are the dominant
  false-positive sources; hence the conservative default policy and `--strict`
  opt-in.

## Development

The repository uses [`devenv`](https://devenv.sh) for the default shell and a
Nix flake for the package, checks, and apps. Both are pinned to the same
revisions as the working `hoard` scaffold in this workspace.

```console
$ direnv allow              # or: devenv shell
$ devenv test               # fmt + clippy + tests
$ cargo run -p pvpdiff-cli -- --version

$ nix build .#default
$ ./result/bin/pvpdiff --version

# Local mode needs GHC/cabal; they are deliberately not in the default shell:
$ nix develop .#local
```

`nix flake check` in a read-only sandbox can be evaluated with a redirected
cache (`HOME=/tmp/nixhome XDG_CACHE_HOME=/tmp/nixhome/.cache`); a full
`nix build` needs a writable Nix cache. Details are in
[development](docs/development.md).

## Repository layout

| Path | Contents |
|---|---|
| `crates/pvpdiff-parse` | normalized API model, Hoogle `.txt` parser, Cabal adapter |
| `crates/pvpdiff-core` | normalization, diff, PVP rules, version math, JSON/SARIF/Markdown |
| `crates/pvpdiff-hackage` | Hackage URLs, blocking HTTP client, ETag cache, index reader |
| `crates/pvpdiff-cli` | the `pvpdiff` binary |
| `docs/` | feasibility record, design, PVP rules, contracts, limits, roadmap |
| `schemas/` | committed JSON Schemas (generated and drift-tested) |
| `fixtures/` | synthetic fixtures committed; real Hackage fixtures fetched and SHA-256-pinned |

## Contributing and commit discipline

The project is built in small, green increments. Every commit that changes a
command, rule, schema, or policy updates `CHANGELOG.md` in the same commit; see
the [development guide](docs/development.md) for the checklist. `main` should
always build and test.

## Licence

Apache-2.0. See [LICENCE](LICENCE).
