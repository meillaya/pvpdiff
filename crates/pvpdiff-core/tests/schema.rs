//! Schema drift test: the committed report schema must match the Rust types.
//!
//! The test regenerates the schema from `Report` and compares the pretty JSON
//! byte for byte with the committed file. A mismatch means the contract changed
//! without an explicit schema commit.

use std::path::PathBuf;

use pvpdiff_core::report::Report;

/// Check the committed schema against the generated one.
#[test]
fn committed_report_schema_matches_types() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/pvpdiff-v1.schema.json");
    let committed = std::fs::read_to_string(&path).expect("read committed schema");
    let generated = Report::schema_value();
    let mut generated_text = serde_json::to_string_pretty(&generated).expect("serialize schema");
    generated_text.push('\n');
    assert_eq!(
        committed, generated_text,
        "schemas/pvpdiff-v1.schema.json is out of date; regenerate it with `pvpdiff schema`"
    );
}
