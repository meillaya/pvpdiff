# Survey dataset

`survey-v1.jsonl.gz` is the gzipped JSONL produced by the P4 Hackage survey
described in [docs/pvp-in-the-wild.md](../docs/pvp-in-the-wild.md).

- Uncompressed size: 10,000 records.
- Uncompressed SHA-256:
  `6f1d5fd692d51466a6c3c986f3989d916318bd96e16da81c8b66d33be9bfeb13`.
- Compressed SHA-256:
  `3b67df10c741fb1c282b216954a0de1e9e8ee7909f93a138ba91d7040d2ee838`.
- Record schema: [schemas/survey-v1.schema.json](../schemas/survey-v1.schema.json).
- Index snapshot SHA-256:
  `5e2a6064612825068b1d34114b95cf6c42d70e56ddc610cf839c520e8c71f80c`.

Each line is one analyzed or skipped package record. The classifier and record
shape are version 1. The dataset is derived from public Hackage metadata; it
does not contain source code or documentation text.
