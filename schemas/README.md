# JSON Schemas

Committed schemas are generated from the Rust report types with `schemars`:

- `pvpdiff-v1.schema.json`: the `diff` report (P1)
- `blast-radius-v1.schema.json`: reverse-dependency bounds (P3)
- `survey-v1.schema.json`: one survey record (P4)

`pvpdiff schema` prints the same content. A test compares the committed files to
the generated output so schema changes are always an explicit commit with a
changelog entry. See [docs/report.md](../docs/report.md) for the field contract
and determinism rules.
