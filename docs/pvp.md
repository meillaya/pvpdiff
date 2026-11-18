# PVP rules used by pvpdiff

Sources: [pvp.haskell.org](https://pvp.haskell.org/), the
[PVP FAQ](https://github.com/haskell/pvp/blob/master/pvp-faq.md), and the open
ambiguity issues [#65](https://github.com/haskell/pvp/issues/65) (type
constraint changes), [#56](https://github.com/haskell/pvp/issues/56) (changes
the PVP does not enumerate), and [#51](https://github.com/haskell/pvp/issues/51)
(upper bounds).

`pvpdiff` states an interpretation and reports candidates. It does not certify a
release. The default policy is conservative; `--strict` applies the literal
reading.

## 1. Version semantics

- A version is a non-empty sequence of numeric components. `A.B.C` is the
  recommended shape; extra components are unregulated.
- **A.B is the major version; C is the minor version.** Adding API requires a
  greater C; breaking API requires a greater A.B. The PVP does **not** special-case
  `0.x`.
- Version ordering is component-wise numeric, not SemVer: `2.0.1.0 > 2.0.1`.
  The tool implements a dedicated `HaskellVersion` type; it never uses the
  `semver` crate for Haskell versions.
- `observedBump` = comparison of `A.B` (major), then `C` (minor), then extras
  (`other`), or `none`.

The [PVP FAQ](https://github.com/haskell/pvp/blob/master/pvp-faq.md) explicitly
rejects a SemVer-style "0.x is unstable" exception.

## 2. Assessment

`pvp.assessment` has three values:

| Value | Meaning |
|---|---|
| `consistent` | the observed version bump is not contradicted by the classified changes (not proof of compliance) |
| `violation-candidate` | confident breaking/additive changes require a larger bump than observed |
| `unknown` | the diff contains unknowns, docs are missing, parsing failed, or inputs are inconsistent |

Rules:

- `classification = breaking` → expected major. A minor/other/none observed bump
  is a `violation-candidate`.
- `classification = additive` → expected at least minor. `other`/`none` is a
  `violation-candidate`; major is permitted.
- `classification = none` → any bump is consistent; a patch is sufficient.
- Any `unknown` (and no breaking) → `unknown`.
- The tool never says "PVP-compliant". A clean report means "no evidence of
  breaking changes in the available data".

## 3. Suggested next version

Canonical three-component suggestions:

| Classification | Suggestion | Rationale |
|---|---|---|
| breaking | `A.(B+1).0` | major bump; resets the minor component |
| additive | `A.B.(C+1)` | minor bump; keeps the major version |
| none | current version | no API change; patch/no bump |
| unknown | `null` | review; if confirmed breaking, use `A.(B+1).0` |

Extra components are unregulated by the PVP and are zeroed in the suggestion;
the report's rationale says so. Maintainers who deliberately use a fourth
component can append their own scheme.

## 4. Rule catalog

Rule IDs are stable and appear in JSON, text output, and SARIF. "Default" is
the policy without `--strict`; "Strict" is the PVP-literal mode.

| Rule | What changed | Default | Strict | Basis / caveat |
|---|---|---|---|---|
| `pvp/module-removed` | exposed module absent in the new version | breaking | breaking | removed entity |
| `pvp/module-added` | new exposed module | additive | additive | adding modules is normally minor; module name collisions caveat |
| `pvp/reexport-changed` | Cabal `reexported-modules` differs | breaking/additive | same | re-exported API |
| `pvp/entity-removed` | function/type/class/pattern no longer exported | breaking | breaking | removed entity |
| `pvp/entity-added` | new exported entity | additive | additive | new binding/type/class |
| `pvp/entity-kind-changed` | e.g. `data` → `newtype`, function → pattern | breaking | breaking | definition changed |
| `pvp/signature-changed` | parsed types differ confidently | breaking | breaking | entity type changed |
| `pvp/signature-unverifiable` | differs but aliases/families/parser prevent confidence | unknown | unknown | data does not prove the change |
| `pvp/data-declaration-changed` | data/newtype head, parameters, or kinds differ | breaking | breaking | datatype definition changed |
| `pvp/constructor-added` | constructor added | breaking | breaking | datatype definition changed; exhaustiveness |
| `pvp/constructor-removed` | constructor removed | breaking | breaking | removed entity |
| `pvp/constructor-changed` | constructor field types/order/strictness differ | breaking | breaking | datatype definition changed |
| `pvp/constructor-order-changed` | same constructors, different order | unknown | unknown | derived `Enum`/`Bounded` semantics; PVP does not enumerate |
| `pvp/field-added` / `-removed` / `-changed` | record selector added/removed/retyped | breaking | breaking | record definition changed |
| `pvp/type-synonym-changed` | RHS differs after normalization | breaking | breaking | definition changed |
| `pvp/type-family-changed` | family declaration/injectivity changed | unknown | unknown | equations and instances are not in Hoogle data |
| `pvp/class-method-added` | signature heuristically attributed to a class | unknown | breaking | a default body would make it additive; Hoogle does not show it |
| `pvp/class-method-removed` | method signature removed | breaking | breaking | class definition changed/removed entity |
| `pvp/superclass-added` / `-removed` | class context differs | breaking | breaking | class definition changed |
| `pvp/class-declaration-changed` | kinds/parameters/associated types differ | breaking | breaking | class definition changed |
| `pvp/pattern-added` / `-removed` / `-changed` | pattern synonym entries | additive / breaking / breaking | same | entity add/remove/type change |
| `pvp/instance-added` | new instance in the package | additive (orphan heuristic: breaking) | same | non-orphan instances are minor; orphan instances are major per PVP |
| `pvp/instance-removed` | instance no longer listed | unknown | breaking | PVP says major, but doc-build GHC moves create false removals |
| `pvp/instance-changed` | instance head/context differs | unknown | breaking | could be remove+add; renderer noise |
| `pvp/fixity-changed` | fixity line added/removed/associativity/precedence changed | unknown | breaking | PVP does not enumerate fixity (#56); parsing risk is real |
| `pvp/deprecation-added` | doc-text `Deprecated:` on an existing entity | unknown | breaking | PVP says deprecation SHOULD count as removal; detection is partial |
| `pvp/deprecation-removed` | deprecation flag removed | none | none | no bump required |
| `pvp/whoops-stub` | signature uses the `Whoops "..."` error type | unknown | unknown | deliberate error stub; needs human review |
| `pvp/docs-unavailable` | Hoogle `.txt` missing/non-200 | report-level unknown | same | cannot diff |
| `pvp/renderer-drift-suspected` | content era or mass-change guard fires | downgrades affected changes to unknown | same | Hoogle records no GHC/Haddock version |
| `pvp/re-export-only-module` | module has no named entries | diagnostic only | same | its surface is not observable in Hoogle data |

### 4.1 Why the default is conservative

The [feasibility record](feasibility.md) measured the failure modes. The two
biggest:

- **Internal modules.** Core libraries expose `*.Internal` modules, so cabal
  filtering does not remove them. In the text 2.0.1→2.0.2 patch pair, the only
  "changed" signatures were primop shims in `Data.Text.Internal.PrimCompat`;
  reporting those as breaking would cry wolf on a legal release. Default:
  report, do not count; `--strict` applies the PVP FAQ's letter.
- **Instances.** Different doc-build GHC/base versions move or rename module
  qualifications (`GHC.IsList`→`GHC.Exts`, `Data.Void`→`GHC.Base`) and hide
  CPP-dependent instances. Default: removals are `unknown`; additions are
  additive unless the orphan heuristic fires.

### 4.2 What is not derivable

Class-method defaults, roles, deriving clauses, closed type-family equations,
type/data family instances, pattern-synonym bidirectionality, external-type
instances, behaviour-only changes, and undocumented deprecations are not in the
Hoogle data. Each is either `unknown` or a documented limitation; see
[limitations](limitations.md). The classifier must not infer a breaking change
from the absence of data.

## 5. Changelog skeleton

`--format markdown` renders the analysed package's API changes as a changelog
skeleton, grouped by `Breaking`, `Additive`, `Unknown: review`, and
`Internal (not counted)`, with modules and signatures. It is a starting point
for the maintainer's own `CHANGELOG.md`, not a replacement; behaviour changes
must be added by hand. This is separate from pvpdiff's own changelog policy in
[development](development.md).
