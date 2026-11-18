//! Schema drift test for survey JSONL records.
//!
//! The test regenerates the schema from `record_schema` and compares it with
//! the committed file.

use std::path::PathBuf;

/// Check the committed survey schema against the generated one.
#[test]
fn committed_survey_schema_matches_types() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/survey-v1.schema.json");
    let committed = std::fs::read_to_string(&path).expect("read committed schema");
    let generated = pvpdiff_survey::record_schema();
    let mut text = serde_json::to_string_pretty(&generated).expect("serialize schema");
    text.push('\n');
    assert_eq!(
        committed, text,
        "schemas/survey-v1.schema.json is out of date; regenerate it"
    );
}
