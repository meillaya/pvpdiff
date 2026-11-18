//! Schema drift test for the blast-radius report.
//!
//! The test regenerates the schema from `BlastReport` and compares it byte for
//! byte with the committed file.

use std::path::PathBuf;

use pvpdiff_index::BlastReport;

/// Check the committed blast-radius schema against the generated one.
#[test]
fn committed_blast_schema_matches_types() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/blast-radius-v1.schema.json");
    let committed = std::fs::read_to_string(&path).expect("read committed schema");
    let generated = BlastReport::schema_value();
    let mut text = serde_json::to_string_pretty(&generated).expect("serialize schema");
    text.push('\n');
    assert_eq!(
        committed, text,
        "schemas/blast-radius-v1.schema.json is out of date; regenerate it"
    );
}
