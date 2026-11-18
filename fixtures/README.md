# Fixtures

Three tiers, mirroring the `hoard` project's discipline:

| Tier | Committed | Contents |
|---|---|---|
| Synthetic | yes | hand-written Hoogle `.txt`/`.cabal` inputs covering parser edge cases; generated expected reports where useful |
| Expected | yes | curated classifications/rule sets for the eight golden pairs, not raw Hackage data |
| Real | no (gitignored) | the eight Hackage pairs, downloaded by `scripts/fetch-fixtures.sh` and verified by SHA-256 |

## Golden pairs

The P1 pair list is in [docs/testing.md](../docs/testing.md). The manifest
`fixtures/pairs.tsv` is added in P1 with columns:

```
package  from  to  sha256(from docs)  sha256(to docs)  sha256(from cabal)  sha256(to cabal)
```

The fetch script refuses to write unverified fixtures: empty digest cells are an
error. Tests that need `fixtures/real/` print a skip notice and pass when the
directory is absent, so a clean checkout and the Nix sandbox work offline; CI
fetches the real set explicitly.

## Licensing

Raw Hackage documentation and Cabal files remain under their packages'
licences. They are downloaded locally and are not committed or redistributed by
this repository. The curated expected files contain only classifications and
short API names for test assertions.
