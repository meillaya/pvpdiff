# Design

`pvpdiff` compares two versions of one Haskell package using Hackage's
per-version Hoogle text databases, normalizes their exported API surfaces, and
classifies the differences against the Package Versioning Policy. It never
compiles the package in Hackage mode and never uses GHC interface files.

## 1. Principles

1. **No GHC/HIE coupling in the Hackage path.** `.hie` is locked to the exact
   GHC build; Hoogle `.txt` is generated per release and already hosted by
   Hackage. Local mode is the only place that shells out to GHC/cabal.
2. **Report candidates, not proof.** The data is lossy (see
   [feasibility](feasibility.md) and [limitations](limitations.md)). The default
   output is tri-state with an explicit `unknown` bucket; the tool never claims
   PVP compliance.
3. **Staged normalization.** Compare raw text first, then parsed/AST-normalized
   types, then synonym-expanded types. If a comparison needs information the
   data does not contain, return `unknown` rather than inventing a break.
4. **Deterministic artifacts.** No timestamps in reports, sorted arrays, stable
   rule IDs, committed schemas, byte-identical snapshot tests.
5. **Advisory by default.** Findings do not fail a process or a workflow unless
   the caller opts in with `--fail-on` (exit code 100, cargo-semver-checks
   style).
6. **Provenance is part of the result.** Every report records the URLs and
   SHA-256 digests of the inputs it used, plus the embedded `@package`/`@version`
   verification.

## 2. Crates and boundaries

| Crate | Responsibility | Depends on | Must not know about |
|---|---|---|---|
| `pvpdiff-parse` | normalized model, Hoogle `.txt` parser, Cabal adapter | `cabalist-parser`, `serde`, `thiserror` | network, PVP rules, output formats |
| `pvpdiff-core` | normalization, diff, classification, version math, JSON/SARIF/Markdown/Text reports | `pvpdiff-parse`, `serde`, `serde_json`, `schemars`, `serde-sarif` | HTTP, cache paths, CLI parsing |
| `pvpdiff-hackage` | URL layout, blocking HTTP, ETag/content cache, `01-index.tar.gz` reader, Stackage pins | `reqwest`, `flate2`, `tar`, hashes, `directories` | Hoogle/Cabal parsing, PVP semantics |
| `pvpdiff-cli` | argument parsing, orchestration, progress, exit codes | all of the above, `clap`, `anyhow`, `indicatif` | business rules |

The arrow is one-directional: CLI → {core, hackage}; core → parse; hackage is
independent. `pvpdiff-hackage` returns bytes/paths so the parser stays testable
offline.

## 3. Data flow (Hackage mode)

```
pkg + from/to version
        │
        ├─ pvpdiff-hackage: resolve versions, fetch docs/<pkg>.txt + <pkg>.cabal
        │                   (ETag/content cache; SHA-256 recorded)
        │
        ├─ pvpdiff-parse: ApiSurface { package, version, modules }
        │     ├─ Hoogle line-state parser → modules, entities, instances, fixities
        │     └─ Cabal adapter → exposed modules, reexports, conditionals, deps
        │
        ├─ pvpdiff-core: normalize → diff → classify → group/dedupe
        │     ├─ verdict + confidence + PVP assessment + suggested version
        │     └─ report JSON / text / Markdown changelog skeleton / SARIF
        │
        └─ pvpdiff-cli: stdout document, stderr progress, exit code
```

## 4. Parsing model

`ApiSurface` is a map of module name → `ModuleApi`. A `ModuleApi` holds ordered
entity records keyed by `(kind, name)`, a package-global instance list, and
metadata such as "this module has no named entries" (a re-export-only module).

Entity kinds: function/value, data, newtype, type synonym, type family,
data family, class, pattern synonym, constructor, record field. Data types own
their constructors/fields; classes own methods (heuristically) and associated
types. Fixities and deprecation flags attach to entities.

The parser is a line state machine, not a blank-line-delimited format: Haddock
emits adjacent entries, interleaved doc blocks, and class `where { ... }`
blocks. It validates the header and the `@package`/`@version` lines before
trusting a file. Docs are parsed only for `Deprecated:`/`Since:` flags and are
otherwise excluded from the semantic model.

The Cabal adapter wraps `cabalist-parser` and owns four gaps: `reexported-modules`
extraction, common-stanza resolution, conditional policy (union deps; unknown
module surface when flags/impl/os guard exposed/reexported modules), and old
`cabal-version` tolerance. Unparseable input yields `Unknown { reason }`, never
a panic.

## 5. Normalization and diff

Stages, cheapest first:

1. **Exact text** after whitespace collapse, doc removal, pragma stripping, and
   `*`/`Type` normalization → equal means no change.
2. **Alpha-equivalence**: rename type variables in order of first appearance,
   drop `forall` binder names, strip `:: Type` binders, sort top-level
   constraints, normalize `$sel:` selectors.
3. **Synonym expansion** using the two databases' own `type` entries (depth
   limit 8). If expansion is recursive/unparseable or a type family is involved,
   the result is `unknown`.
4. **AST comparison** for the Haskell type subset. Differences in arity,
   argument order, constraints, result type, constructor fields, or strictness
   are confidently `breaking`; anything the parser cannot classify falls back
   to `unknown`.

The diff runs per module. Identical changes that appear in several
re-exporting modules are grouped into one change with a `modules` list; data
type/class add/remove aggregates its children into one change. Instances are
compared package-globally (module attribution is not stable across doc builds).

Two guards downgrade to `unknown`:

- **Renderer drift:** different-era formatting or a suspicious mass change
  (>20% of shared signatures).
- **Refactor guard:** a module loses all entries but every entity still exists
  elsewhere (likely a switch to a wholesale `module X` re-export).

## 6. Policy

The default policy is tuned for signal quality; `--strict` applies the literal
PVP reading. The full table is in [pvp](pvp.md). Summary:

| Signal | Default | `--strict` |
|---|---|---|
| changes in `Internal`/`Private`-named modules | reported, not counted | counted |
| removed/changed instances | `unknown` | `breaking` |
| added orphan instance | `breaking` (heuristic) | same |
| fixity change | `unknown` | `breaking` |
| deprecation added | `unknown` | `breaking` |
| class method added | `unknown` (default body unknown) | `breaking` |
| removed/renamed export, changed signature, constructor/field change | `breaking` | same |

This is a product decision, not a claim that the strict reading is wrong; the
[PVP FAQ](https://github.com/haskell/pvp/blob/master/pvp-faq.md) says exposed
internals are covered. The rationale is documented in [pvp](pvp.md) and
[limitations](limitations.md).

## 7. Output and exit codes

One report model serializes to text, JSON (`schema: 1`, deterministic), a
Markdown changelog skeleton, and SARIF 2.1.0. The JSON contract is in
[report](report.md).

Exit codes: `0` analysis completed; `1` tool/IO/network error; `100` gate
triggered by `--fail-on`; Clap's `2` remains usage error. SARIF upload is the
caller's step; findings never fail CI by default.

## 8. Caching and index

The client cache stores immutable Hoogle `.txt` files forever and revalidates
mutable `.cabal` files (Hackage serves the latest revision). Provenance hashes
go into the report. The reverse-dependency index is a separate, explicit sync:

```
pvpdiff index sync   # conditional GET 01-index.tar.gz; stream, parse, last revision wins
pvpdiff index status
```

It writes a JSONL dependency database plus metadata (index ETag/digest, counts);
no SQLite dependency. Blast radius loads the database and evaluates Cabal
version ranges for dependents of the target package. Stackage is a later join
of snapshot pins with the same data.

## 9. Explicit non-goals

No GHC/HIE in Hackage mode, no source parsing, no semantic/behaviour checks, no
role or family-equation detection, no automatic version bumping, no Hackage
server enforcement, no Stackage solver. These follow from the data and from the
advisory positioning; they are listed in [limitations](limitations.md).

## 10. Deliverable shape

The repository is a four-crate Cargo workspace with a `devenv.nix` development
shell and a `flake.nix` exposing the package, dev shells (including an optional
GHC shell for local mode), checks, apps, and formatter. CI runs fmt, clippy,
tests, docs, flake checks, Nix build, `cargo-deny`, and actionlint. The
changelog is updated in the same commit as each user-visible change; see
[development](development.md).
