# PVP in the wild: a Hackage survey

This report classifies the API change between the newest two versions of 5,940
Hackage packages. It uses the same Hoogle-based classifier as `pvpdiff diff`,
the default conservative policy, and the cached Hackage index snapshot below.

The dataset is committed as `survey/survey-v1.jsonl.gz`. The SHA-256 of the
uncompressed JSONL is
`6f1d5fd692d51466a6c3c986f3989d916318bd96e16da81c8b66d33be9bfeb13`; the
compressed file is
`3b67df10c741fb1c282b216954a0de1e9e8ee7909f93a138ba91d7040d2ee838`.

## Method

- Index snapshot: `01-index.tar.gz`, 19,713 packages and 155,776 versions,
  SHA-256 `5e2a6064612825068b1d34114b95cf6c42d70e56ddc610cf839c520e8c71f80c`.
- Selection: the first 10,000 package names in sorted order, with the newest
  versions probed for Hoogle databases, newest first, up to three versions.
  This is an alphabetical slice, not a random sample.
- Pair: the newest two versions with a Hoogle database. If fewer than two
  databases exist, the package is recorded as skipped.
- Classification: `pvpdiff diff` with default options. The aggregate class is
  `breaking`, `additive`, `none`, or `unknown`; `unknown` covers missing data,
  synonym or family uncertainty, added class methods, instance removals, and
  other cases the data cannot decide.
- No package was compiled and no Hackage data was modified.

## Dataset

| Measure | Value |
|---|---:|
| Package names selected | 10,000 |
| Analyzed pairs | 5,940 |
| Skipped packages | 4,060 |
| Docs coverage | 59.4% of selected packages |
| Indexed packages | 19,713 |
| Indexed versions | 155,776 |

Skips: 4,052 packages had fewer than two Hoogle databases, 6 had a transient
fetch error, and 2 had a database that did not parse.

## Findings

1,146 of the 5,940 analyzed pairs are violation candidates, 19.3%. The default
classifier reported 2,255 breaking pairs, 555 additive pairs, 2,766 pairs with
no counted public API change, and 364 unknown pairs. The counts do not add to
1,146 because a violation candidate is an assessment, not a classification.

The most frequent counted change rules were:

| Rule | Counted changes |
|---|---:|
| `pvp/instance-added` | 235,891 |
| `pvp/entity-added` | 58,203 |
| `pvp/instance-removed` | 26,923 |
| `pvp/signature-unverifiable` | 22,369 |
| `pvp/module-added` | 21,403 |
| `pvp/entity-removed` | 13,281 |
| `pvp/signature-changed` | 6,614 |
| `pvp/module-removed` | 6,412 |
| `pvp/constructor-changed` | 4,839 |
| `pvp/field-added` | 4,458 |

Instance additions dominate because they are numerous per release and are
classified as additive. `signature-unverifiable` is large because the
conservative policy refuses to call a signature change breaking when a type
synonym, type family, primitive type, or renderer-era difference prevents a
safe comparison.

The 1,146 violation candidates split into releases whose observed bump was too
small for a confident breaking or additive classification. A candidate is a
review prompt, not a citation. The classifier cannot see behaviour-only
changes, roles, family equations, default method bodies, or external-type
instances, and the default policy reports exposed `Internal` modules without
counting them.

## Observed bumps and assessments

| Observed bump | Pairs |
|---|---:|
| major | 1,947 |
| minor | 2,197 |
| other-component | 1,796 |
| none | 0 |

| Assessment | Pairs |
|---|---:|
| consistent | 4,430 |
| violation-candidate | 1,146 |
| unknown | 364 |

The large `other-component` group is mostly four-component versions where the
first three PVP components did not change. The tool treats those as PVP "other"
changes, not as a minor bump.

## What these numbers do not mean

- They are not a compliance audit of Hackage. Coverage is 59.4% for this slice,
  and the classifier uses a conservative `unknown` bucket by design.
- The slice is alphabetical, so it over-represents names early in the alphabet.
  A random or full run would give different rates; `survey run` supports
  sharding and resuming for that.
- Instance churn from doc-build GHC differences is reported as `unknown` on
  removals. Treat the instance totals as lower-confidence than entity or module
  changes.
- The report counts direct library bounds in the index, not test-suite or
  benchmark dependencies.

## Reproduce

```console
$ pvpdiff index sync --cache-dir .cache/pvpdiff
$ pvpdiff survey run --limit 10000 --jobs 12 --delay-ms 75 \
    --max-versions 3 --out data/survey-v1.jsonl --cache-dir .cache/pvpdiff
$ pvpdiff survey report --input data/survey-v1.jsonl \
    --out docs/pvp-in-the-wild.md
```

The one-page summary is this file. The JSONL records are the dataset; the
schema is `schemas/survey-v1.schema.json`.
