# GitHub Action

The repository ships a composite action that installs a release binary and runs
`pvpdiff check`. It expects the caller to provide GHC and cabal, because the
local check builds the package's Haddock output. The action does not upload
SARIF; that step stays in the caller's workflow so the permissions and fork
behavior are explicit.

## Inputs

| Input | Default | Meaning |
|---|---|---|
| `package-dir` | `.` | Directory containing the package `.cabal` file |
| `against` | `latest` | Hackage baseline version, `latest`, or `previous` |
| `args` | empty | Extra `pvpdiff check` arguments |
| `sarif-file` | `pvpdiff.sarif` | SARIF output path |
| `json-file` | `pvpdiff.json` | JSON report output path |
| `version` | `latest` | pvpdiff release tag to install |
| `fail-on` | `never` | `never`, `breaking`, `unknown`, or `any` |

Outputs: `classification`, `json-file`, `sarif-file`.

## Workflow

```yaml
name: PVP API diff

on:
  pull_request:
  push:
    branches: [main]

permissions:
  contents: read
  security-events: write

jobs:
  pvpdiff:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v7

      - uses: haskell-actions/setup@v2
        with:
          ghc-version: "9.10"
          cabal-version: "3.14"

      - uses: meillaya/pvpdiff@v1
        with:
          package-dir: .
          against: latest

      - uses: github/codeql-action/upload-sarif@v4
        continue-on-error: true
        with:
          sarif_file: pvpdiff.sarif
          category: pvpdiff
```

## Behavior

- `pvpdiff check` runs `cabal haddock --haddock-hoogle`, finds the database for
  the local package version, compares it with the chosen Hackage baseline, and
  writes JSON plus SARIF.
- Findings are advisory. The action exits 0 unless `fail-on` is set. Exit code
  100 is the gate signal.
- Fork pull requests receive a read-only token and cannot upload SARIF. Run the
  analysis on `push` or `schedule`, or keep the JSON report as an artifact.
- Code scanning must be enabled. Keep `continue-on-error: true` on the upload
  step if repositories without code scanning share the workflow.
- Pin the action ref and the `version` input together. Release assets are
  produced by cargo-dist; do not hand-edit the generated release workflow.

## Prerequisites and limits

- The runner needs GHC and cabal. The example uses `haskell-actions/setup`;
  pin a version your package supports.
- Multi-library packages currently compare the main library; others are
  reported as unknown in a later phase.
- A local GHC different from the Hackage doc-build GHC can produce
  renderer-drift diagnostics. The report says so; it does not hide the
  uncertainty.
