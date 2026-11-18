# Feasibility record: Hoogle-based PVP API diff

**Status:** research complete, accepted for P1. This document records the
evidence behind the design; the implementation plan lives in
[roadmap](roadmap.md) and the rule semantics in [pvp](pvp.md).

**Verdict: GO**, with a narrower promise than the original pitch. `pvpdiff` is
an advisory **breaking-change candidate detector + PVP bump adviser**, not a
PVP-compliance certifier. The cheap go/no-go passed 5/5 on aggregate severity
after modest normalization, and the remaining false-positive classes have
specific fixes. The honest unowned gap is the PVP workflow (rule engine,
suggested version, changelog, SARIF/Action, reverse-dependency blast radius),
not API diffing itself.

## 1. Method

All version pairs were verified live on Hackage on 2026-11-17/18: both
`docs/<pkg>.txt` URLs returned 200, the embedded `@package`/`@version` matched,
the `.cabal` files were fetched, and the source tarballs provided the
`CHANGELOG` used as ground truth. A throwaway Python prototype parsed the
Hoogle entries, split modules into primary and `Internal`/`Private` groups,
dropped docs, normalized obvious noise, and emitted removed/added/changed
entities. The source of truth for format semantics was Haddock's
[Hoogle backend](https://github.com/haskell/haddock/blob/master/haddock-api/src/Haddock/Backends/Hoogle.hs)
and 21 downloaded databases (15 modern, 6 older).

## 2. Go/no-go results (five verified pairs)

`R/A/C` = removed/added/changed top-level items. "Primary" excludes
`Internal`/`Private`-named modules and counts after normalization; "internal"
is reported separately.

| Pair | PVP bump | Naive R/A/C | Normalized primary R/A/C | Verdict | Changelog comparison |
|---|---|---|---|---|---|
| [`text-2.0.1`](https://hackage.haskell.org/package/text-2.0.1/docs/text.txt) → [`2.0.2`](https://hackage.haskell.org/package/text-2.0.2/docs/text.txt) | minor | 0/47/6 | 0/13/0 (34/6 internal) | ADDITIVE | MATCH: 12 documented additions recovered; the 6 "changed" entries are primop shims in `Data.Text.Internal.PrimCompat`. The naive diff would falsely accuse a legal patch release. |
| [`containers-0.6.7`](https://hackage.haskell.org/package/containers-0.6.7/docs/containers.txt) → [`0.7`](https://hackage.haskell.org/package/containers-0.7/docs/containers.txt) | major | 6/19/3 | 1/2/2 (real 0/2/1) | BREAKING | MATCH: 3/3 changelog API items (`CyclicSCC` constructor→pattern, `NECyclicSCC` added, `IntSet.fromRange` added). Two primary FPs + 23 internal-module items. |
| [`aeson-2.1.2.1`](https://hackage.haskell.org/package/aeson-2.1.2.1/docs/aeson.txt) → [`2.2.0.0`](https://hackage.haskell.org/package/aeson-2.2.0.0/docs/aeson.txt) | major | 65/45/34 | 22/26/17 unique | BREAKING | MATCH on verdict; the diff also finds undocumented `KeyValue` arity/`(.=)` changes. Two instance FPs; external-type instance changes are invisible. |
| [`bytestring-0.11.5.3`](https://hackage.haskell.org/package/bytestring-0.11.5.3/docs/bytestring.txt) → [`0.12.0.0`](https://hackage.haskell.org/package/bytestring-0.12.0.0/docs/bytestring.txt) | major | 0/27/4 | 0/22/2 | BREAKING | MATCH: `data ShortByteString`→`newtype`, `SBS` constructor→pattern, 22 documented additions. A removal-only heuristic would wrongly say ADDITIVE. |
| [`text-1.2.5.0`](https://hackage.haskell.org/package/text-1.2.5.0/docs/text.txt) → [`2.0`](https://hackage.haskell.org/package/text-2.0/docs/text.txt) | major | 38/53/62 | 28/34/43 | BREAKING | MATCH near item-level: constructor renames, `copyI` argument order, `Word16`→`Word8`, module removals, `HasCallStack` on 33 partial functions, etc. |

**P1 gate:** add three minor-bump pairs (aeson 2.1.1.0→2.1.2.0,
containers 0.6.6→0.6.7, lens 5.2.2→5.2.3) before claiming the detector is
calibrated; the original five over-sample major bumps.

### 2.1 False-positive catalogue (with fixes)

| Class | Example | Fix |
|---|---|---|
| Exposed `*.Internal` modules | text 2.0.2's only changed sigs are `word16ToWord#` in `Data.Text.Internal.PrimCompat`; containers adds `Stack`/`Push` in `Data.Map.Internal` | Parse and report internal-named modules with `counted: false`; `--strict` counts them |
| Doc-build instance churn | containers 0.7 still has `Foldable1 Tree` under CPP but the DB omits it; `GHC.IsList` vs `GHC.Exts`; aeson instances moved to hidden modules | Compare instances package-globally with last-component normalization; removals are `unknown`, not `breaking` |
| Type-synonym restatement | containers `dfs :: ... -> Forest Vertex` vs `... -> [Tree Vertex]` | Expand synonyms from the DB's own `type` entries; otherwise `unknown` |
| Renderer artifacts | `newtype () => Identity a`, `_` hole types, `$sel:` field names, `*`/`Type` | Normalize empty contexts, pragmas, kind spelling, selector names |
| Re-export double counting | aeson `KeyValue`/`(.=)` counted twice; bytestring `readInt*` in strict and lazy modules | Group identical changes across modules; keep module list and per-module add/remove |
| `Whoops "..."` stubs | containers `insertWith'` entries rendered with an error-message type | Dedicated `pvp/whoops-stub` unknown rule |

### 2.2 Structural false negatives (no docs diff can see them)

- Behaviour-only changes: bytestring `readInt` overflow→`Nothing`, `fromListN`
  exception semantics, aeson `omitNothingFields`, text UTF-8 replacement.
- Instances for external types: aeson's `URI`/`Down` additions and attoparsec
  instance removals are absent from both DBs.
- Roles, pattern-synonym bidirectionality, class-method default bodies, closed
  type-family equations, deriving strategies.
- Deprecations that are not in doc text; only a `-- | <i>Deprecated:</i>` block
  is visible.

Therefore the tool reports `unknown` where it cannot decide and never claims
compliance. The go/no-go is asymmetric: it can flag violation candidates, it
cannot certify a release.

## 3. What the Hoogle `.txt` database actually is

Fetched shape (Hackage, all versions are one URL each):

```
-- Hoogle documentation, generated by Haddock
-- See Hoogle, http://www.haskell.org/hoogle/

-- | <package description...>
@package text
@version 2.0.2
...

-- | <module doc>
module Data.Text

-- | ...
data Text
...
```

- **Stable, line-oriented skeleton.** `module M` starts a section; doc blocks
  (`-- |` then `--   `) attach to the next structural line; entries are one line
  each (`name :: sig`, `data`, `newtype`, `type`, `type family`, `data family`,
  `class`, `pattern`, `instance`, `infix*`); no wrapped signatures. The only
  multi-line construct is a class `where { ... }` block for associated types
  (4-space indented, semicolon-terminated).
- **Contains:** named exports per exposed module; constructors (with `!` and
  `{-# UNPACK #-}`); record selectors as `[field] ::`; GADT constructors as
  `[Ctor] ::`; pattern synonyms; type synonyms with RHS; class heads,
  superclass contexts, methods as flat sigs, associated types; fixities;
  instances (fully qualified); deprecations as doc text.
- **Does not contain:** roles, deriving clauses, `type`/`data` family equations
  or instances, default-method bodies, Haddock/GHC version, import/re-export
  graph, hidden modules, module-level (`module X`) re-export items.
- **`docs/<pkg>.txt` is exactly the package's exposed modules** (verified
  across 12 packages; text 45/45, containers 29/29, aeson 17/17). No
  `other-modules` leak in. Core libraries show internal modules only because
  they expose them.
- **Sections are export lists of named items.** Explicit per-name re-exports are
  duplicated into the re-exporting section (`data Text` appears in four text
  modules), while a wholesale `module X` re-export is dropped. A re-export-only
  module can therefore have **zero entries** (`Data.Map` in containers 0.8.1,
  mtl `Control.Monad.State`, 14 prettyprinter aliases). This is the main blind
  spot for "removed export".
- **Class methods are not marked.** They are ordinary signatures with the class
  context re-added; top-level functions can share that context (`fromJSON` after
  `FromJSON` methods). Any "method added" classification is heuristic and must
  be `unknown` unless a default method can be proven (it cannot).
- **Instances are the noisiest channel.** Fully qualified heads and
  declaring-module attribution; derived/standalone/orphan are indistinguishable.
  GHC module moves and era changes produce large false diffs (base 4.16→4.18:
  27 removed / 105 added; 17 head-groups changed text only).
- **Docs are 78–87% of non-blank lines**, and 30.6% of unchanged entities had
  doc-only churn in text 1.2.5.0→2.0.2. Drop docs from the semantic diff; keep
  only deprecation/`@since` flags.
- **No version stamp** for Haddock/GHC, so format-era differences are inferred
  from content (qualified vs unqualified instances, bracket fields, `where {}`
  style) and mass-change heuristics.

## 4. Coverage and data sources (measured 2026-11-17/18)

### 4.1 Hoogle docs coverage

| Sample | Coverage |
|---|---|
| Random latest versions | 78/100 (78%) |
| Random historical (package, version) | 109/130 (83.8%) |
| **Latest + previous both available** | **54/74 (73.0%)** (at least one: 63/74 = 85.1%) |
| Random LTS-24.67-pinned versions | 90/100 (90%) |

Failures are roughly 73% "no docs directory at all" (404) and 27% "HTML docs but
no Hoogle `.txt`" (old Haddock). Missing docs must produce `unknown` plus a
diagnostic, not a crash. Local mode exists for unpublished code and for the
27–30% where Hackage data cannot answer the question.

### 4.2 Hackage endpoints

| Source | URL | Notes |
|---|---|---|
| Version list | `https://hackage.haskell.org/package/<pkg>.json` | map version→`normal`/`deprecated`; no timestamps/docs |
| Version metadata | `.../<pkg>-<ver>.json` | `uploaded_at`, `metadata_revision`, licence, synopsis |
| Cabal file | `.../<pkg>-<ver>/<pkg>.cabal` | serves the **latest revision** (`x-revision`); per-revision URLs exist |
| Hoogle DB | `.../<pkg>-<ver>/docs/<pkg>.txt` | 301→hackage-content CDN; ETag |
| Full index | `https://hackage.haskell.org/01-index.tar.gz` | 141,163,034 B; 1,055,097,856 B uncompressed; 362,250 tar entries; 202,685 cabal entries; **155,776 versions / 19,713 packages**; 46,909 revision duplicates (last-entry-wins); 736 MB cabal text; `gunzip|/dev/null` 2.06 s; naive Python scan 41–43 s |
| Preferred only | `https://hackage.haskell.org/packages/index.tar.gz` | 32,651,387 B of `<pkg>/preferred-versions`; not useful for bounds |
| Reverse deps | `https://hackage.haskell.org/package/<pkg>/reverse` | HTML only; text has 5,130 dependents; **no bounds**, so not a shortcut for blast radius |

Rate limiting is undocumented: 230 requests at 12-way parallelism saw no 429,
and the default curl User-Agent is accepted. Still: cache immutable docs, use a
descriptive UA, pace requests, and honor `Retry-After`/5xx with backoff.

### 4.3 Stackage

Latest LTS at probe time: lts-24.67 (GHC 9.10.3), 3,401 pins, text 2.1.3;
nightly-2026-11-18 had text 2.1.4. Sources:
`https://www.stackage.org/<snapshot>/cabal.config` (cheap pins) and
[`stackage-snapshots`](https://github.com/commercialhaskell/stackage-snapshots)
YAML. Snapshot pins do not encode dependents' bounds; answer "would this
snapshot exclude the new version" by joining the already-scanned Hackage index
data with the snapshot's exact versions. That is a P3/P4 filter, not a v1
requirement, and no scraping is needed.

## 5. Prior art (verified 2026-11-17)

| Tool | Status | Overlap / difference |
|---|---|---|
| [`hackage-diff`](https://github.com/blitzcode/hackage-diff) | last upstream push 2018-05-02; Hackage 0.1.0.1, 2023 revision; `Cabal <2.2` | Already did "download Hoogle DBs → parse → diff"; dead and unbuildable on modern GHC |
| [`policeman`](https://github.com/kowainik/policeman) | 0.0.0.0 (2020), last push 2021-02-24; GHC 8.8 | PVP next-version via GHC API/HIE; dead |
| [`check-pvp`](https://hackage.haskell.org/package/check-pvp) | 0.0.3, 2019 | Checks import style vs own bounds; different problem |
| [`packdiff`](https://github.com/composewell/packdiff) | active (2026-10-12), used in Streamly CI | Hoogle-file API diff + changelog; no PVP classification/version/SARIF/blast radius |
| [`print-api`](https://github.com/Kleidukos/print-api) | active (2026-10-26), GHC 9.6–9.12 | Golden-file API dumps; GHC-version-pinned, not version-to-version Hackage workflow |
| [`diff-package-api`](https://github.com/Kleidukos/diff-package-api) | Action v0.1.0.1 (2024), dormant | Golden API CI action; no PVP rules |
| [PVP #47](https://github.com/haskell/pvp/issues/47) | closed 2023-02-18 | The Tools section was **removed**, not updated, because entries were dead |
| [cabal PR #12363](https://github.com/haskell/cabal/pull/12363) | open, 2026-09-16 | Proposes an informational packdiff API-diff job in Cabal CI; adoption signal |

The pitch's "no maintained tool exists" is therefore partly false for API
diffing, but true for the end-to-end PVP workflow. The project must say that
plainly and credit prior art.

## 6. PVP interpretation used by pvpdiff

Source: [pvp.haskell.org](https://pvp.haskell.org/) and the
[PVP FAQ](https://github.com/haskell/pvp/blob/master/pvp-faq.md).

- Version numbers are `A.B.C` (extra components unregulated). **A.B is the major
  version; C is the minor version; 0.x has no exception.**
- Breaking: removed entities; changed types or datatype/class definitions; added
  orphan instances; removed instances; **deprecation SHOULD count as removal**.
- Non-breaking: only new bindings/types/classes/non-orphan instances/modules;
  C must increase. Adding a module is normally minor (module-name-collision
  caveat).
- Multi-library packages expose the union of their public libraries.
- Exposed `*.Internal` modules are subject to PVP by the letter of the FAQ, but
  the tool's default policy treats them as report-only to avoid crying wolf;
  `--strict` applies the letter.
- Conditional APIs weaken the version↔API contract; flag them.
- Open ambiguities that the tool must not pretend to resolve: type-constraint
  changes ([#65](https://github.com/haskell/pvp/issues/65)), non-enumerated
  changes such as fixity ([#56](https://github.com/haskell/pvp/issues/56)), and
  upper bounds ([#51](https://github.com/haskell/pvp/issues/51)).

## 7. Product consequences

1. **Tri-state output is mandatory.** `breaking` / `additive` / `none` /
   `unknown`, plus `pvp.assessment` ∈ `consistent` / `violation-candidate` /
   `unknown`. Never "compliant".
2. **Default conservative policy:** internal-named modules uncounted; instance
   removals, deprecations, fixity changes, and added class methods are `unknown`;
   `--strict` opts into the literal PVP reading.
3. **Hackage-only is not enough:** local `check` mode for unpublished code and
   for the 27–30% docs gap; survey reports coverage rather than hiding it.
4. **Blast radius comes from `01-index.tar.gz`**, cached; Hackage's reverse page
   has no bounds and Stackage pins are not bounds.
5. **The Action is advisory by default.** SARIF upload never fails a workflow on
   findings; gating is `--fail-on` with exit code 100.
6. **Positioning:** the value is PVP rule semantics + workflow + dataset, not
   the diff itself; Rust is chosen for a small portable binary, not as a moat.

## 8. Reproduction

The research artifacts (prototype, dumps, coverage samples) were generated in
temporary directories and are not committed. The checks are reproducible with:

```console
# one pair
curl -sSLo old.txt https://hackage.haskell.org/package/text-2.0.1/docs/text.txt
curl -sSLo new.txt https://hackage.haskell.org/package/text-2.0.2/docs/text.txt
curl -sSLo old.cabal https://hackage.haskell.org/package/text-2.0.1/text.cabal
curl -sSLo new.cabal https://hackage.haskell.org/package/text-2.0.2/text.cabal

# docs coverage sample, index metadata
curl -sSLo /dev/null -w '%{http_code}\n' \
  https://hackage.haskell.org/package/base-orphans-0.9.3/docs/base-orphans.txt
curl -sSIL https://hackage.haskell.org/01-index.tar.gz | grep -iE 'content-length|etag'

# Stackage pins
curl -sSL https://www.stackage.org/lts-24.67/cabal.config | head
```

The curated P1 golden-pair expectations live in `fixtures/expected/` once the
fixture fetch script is completed.

## 9. Open questions carried into implementation

- Can the synonym-normalizer + alpha-renaming eliminate the remaining five-pair
  false positives without hiding real breaks? The P1 gate decides.
- Does adding three minor-bump pairs keep the false-BREAKING rate at zero?
- Are instance removals ever safe to report as `breaking` under `--strict`
  without era detection? Default stays `unknown`.
- What fraction of packages are unreachable through Hackage docs, and does local
  mode need multi-library support in P2? Track in survey/support docs.
- Will Hackage/maintainers accept an advisory Action and a pvp.haskell.org
  tools entry after a track record? External, not a P1 blocker.
