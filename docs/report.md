# Report contracts

**Status:** P0 defines the contracts; the types and committed schemas land in
P1 (report), P3 (blast radius), and P4 (survey). The schemas are generated with
`schemars` and drift-tested against the committed JSON files.

## 1. `pvpdiff diff` report, `schema: 1`

```json
{
  "schema": 1,
  "tool": { "name": "pvpdiff", "version": "0.1.0" },
  "package": "text",
  "from": {
    "version": "2.0.1",
    "apiStatus": "known",
    "docsUrl": "https://hackage.haskell.org/package/text-2.0.1/docs/text.txt",
    "docsSha256": "…",
    "cabalUrl": "https://hackage.haskell.org/package/text-2.0.1/text.cabal",
    "cabalSha256": "…",
    "cabalRevision": 2
  },
  "to": { "…": "same shape" },
  "classification": "additive",
  "confidence": "high",
  "counts": {
    "breaking": 0,
    "additive": 13,
    "none": 0,
    "unknown": 0,
    "internal": 40
  },
  "pvp": {
    "observedBump": "minor",
    "assessment": "consistent",
    "suggestedVersion": "2.0.2",
    "rationale": ["additive changes require a greater C"]
  },
  "changes": [
    {
      "id": "c1",
      "rule": "pvp/entity-added",
      "classification": "additive",
      "confidence": "high",
      "kind": "function",
      "module": "Data.Text",
      "modules": ["Data.Text", "Data.Text.Lazy"],
      "name": "decodeASCIIPrefix",
      "parent": null,
      "before": null,
      "after": { "signature": "ByteString -> Maybe (Text, ByteString)" },
      "counted": true,
      "message": "new export"
    }
  ],
  "diagnostics": [
    {
      "code": "docs-unavailable",
      "severity": "warning",
      "message": "no Hoogle database for aeson-0.6.2.1"
    }
  ],
  "inputs": [
    { "kind": "hoogle", "package": "text", "version": "2.0.1", "sha256": "…" }
  ]
}
```

Rules:

- `classification` is `breaking | additive | none | unknown`; `confidence` is
  `high | medium | low`. `counts.internal` is the uncounted internal-module
  total; `internal` changes appear in `changes` with `counted: false`.
- `pvp.assessment` is `consistent | violation-candidate | unknown`; the
  rationale array is stable text, not a timestamped blob.
- `changes` are sorted by `(rule, modules[0], name)`; `modules` is sorted.
  Duplicate entities appearing in re-exporting modules are grouped into one
  change with all modules listed.
- `diagnostics` are sorted by `code`; `inputs` by `(kind, package, version)`.
- No timestamps or absolute paths. `tool.version` comes from `CARGO_PKG_VERSION`.
- `suggestedVersion` is null for `unknown`; the rationale explains the review.

### 1.1 Blast-radius report, `schema: 1` (P3)

```json
{
  "schema": 1,
  "package": "text",
  "newVersion": "2.1.4",
  "source": {
    "indexSha256": "…",
    "indexEtag": "…",
    "packageCount": 19713,
    "versionCount": 155776
  },
  "counts": {
    "excluded": 0,
    "compatible": 0,
    "noUpperBound": 0,
    "overConstrained": 0
  },
  "dependents": [
    {
      "package": "…",
      "version": "…",
      "range": "^>=2.0 && <2.1",
      "component": "library",
      "status": "excluded",
      "reason": "upper-bound-below-new-version"
    }
  ]
}
```

`status` ∈ `excluded | compatible | no-upper-bound | over-constrained`; results
are sorted by package/version. No timestamps; the index digest is the
reproducibility anchor.

### 1.2 Survey record, `schema: 1` (P4)

One JSON object per line in `data/survey-v1.jsonl`, with package, chosen pair,
classification/assessment, counts, rule histogram, input digests, and
diagnostics. The aggregate `survey report` is derived from the JSONL and carries
the methodology/coverage table.

## 2. Determinism and schema discipline

- Reports are generated from structs with fixed field order, pretty-printed with
  two spaces and a trailing newline; arrays are sorted before serialization.
- `x`/`y` input digests are SHA-256 of the exact bytes used, recorded in
  `inputs`; the cache also stores ETag/Last-Modified metadata but the report
  does not depend on fetch time.
- `schemas/pvpdiff-v1.schema.json`, `schemas/blast-radius-v1.schema.json`, and
  `schemas/survey-v1.schema.json` are committed; a test compares them to the
  generated output. `pvpdiff schema` prints the same content.
- Schema changes require a version bump and a changelog entry. Unknown fields
  are not silently dropped by consumers; the schema is additive within v1.

## 3. SARIF 2.1.0

`--sarif` emits a single SARIF 2.1.0 run:

- `tool.driver.name = "pvpdiff"`, version/semanticVersion from the binary, and
  the repository URL as `informationUri`.
- Static rules for the stable rule IDs, with `shortDescription`,
  `fullDescription`, `help`, and `defaultConfiguration.level`
  (`error` for breaking, `warning` for unknown, `note` for additive).
- One result per counted change by default; `--include-internal`/`--sarif-all`
  adds internal results as notes. `message.text` names the package/versions,
  module, entity, and rule; `locations[].physicalLocation.artifactLocation.uri`
  comes from `--sarif-uri`, with a region when a Cabal span is available and
  line 1 otherwise.
- Deterministic `partialFingerprints` derived from `(rule, module, name, before,
  after)`; no `invocations`/timestamps. Result count is capped below GitHub's
  25,000 limit with a diagnostic if truncation happens.

GitHub code scanning accepts SARIF 2.1.0 only; upload is the caller's step (see
[integration](integration.md)).
