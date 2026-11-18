//! JSONL record contract and resume state.
//!
//! The output file is append-only: one JSON object per line, tagged by
//! `outcome`. An analyzed record carries the pair, the classification, the
//! counts, the rule histogram, and the docs digests. A skipped record carries
//! the reason and the versions the probe actually touched. The schema version
//! is part of every line so a reader can reject a file it does not understand
//! instead of guessing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::Path;

use pvpdiff_core::{Assessment, Bump, Classification, Counts};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{SurveyError, ensure_parent};

/// Schema version of one survey record.
pub const RECORD_SCHEMA: u32 = 1;

/// Skip reason for a package with fewer than two Hoogle databases.
pub(crate) const REASON_DOCS: &str = "docs";
/// Skip reason for a docs fetch that failed in a way that does not prove the
/// database is absent.
pub(crate) const REASON_FETCH: &str = "fetch";
/// Skip reason for a fetched Hoogle database that does not parse.
pub(crate) const REASON_PARSE: &str = "parse";

/// SHA-256 digests of the two Hoogle databases a record compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocsDigests {
    /// Digest of the older database.
    pub older: String,
    /// Digest of the newer database.
    pub newer: String,
}

/// One analyzed package pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzedRecord {
    /// Record schema version, always [`RECORD_SCHEMA`].
    pub schema: u32,
    /// Package name.
    pub package: String,
    /// Older version, the PVP baseline.
    pub from: String,
    /// Newer version, the candidate.
    pub to: String,
    /// Aggregate classification of the pair.
    pub classification: Classification,
    /// Version bump between `from` and `to`.
    pub observed_bump: Bump,
    /// Assessment of the bump against the classification.
    pub assessment: Assessment,
    /// Per-class change counts from the diff.
    pub counts: Counts,
    /// Counted changes per rule identifier.
    pub rules: BTreeMap<String, usize>,
    /// Total number of changes, including internal modules that are not
    /// counted.
    pub change_count: usize,
    /// Counted changes classified unknown.
    pub unknown_count: usize,
    /// Digests of the two Hoogle databases.
    pub docs_sha256: DocsDigests,
    /// Diagnostic codes from the diff, sorted and deduplicated.
    pub diagnostics: Vec<String>,
}

/// One package that could not be analyzed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRecord {
    /// Record schema version, always [`RECORD_SCHEMA`].
    pub schema: u32,
    /// Package name.
    pub package: String,
    /// Why the package was skipped: `docs`, `fetch`, or `parse`.
    pub reason: String,
    /// Versions probed for a Hoogle database, newest first.
    pub probed: Vec<String>,
}

/// One survey JSONL record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "lowercase")]
pub enum SurveyRecord {
    /// A pair of versions was analyzed.
    Analyzed(AnalyzedRecord),
    /// No pair could be analyzed.
    Skipped(SkippedRecord),
}

impl SurveyRecord {
    /// Schema version carried in this record.
    pub fn schema(&self) -> u32 {
        match self {
            Self::Analyzed(record) => record.schema,
            Self::Skipped(record) => record.schema,
        }
    }

    /// Package this record is about.
    pub fn package(&self) -> &str {
        match self {
            Self::Analyzed(record) => &record.package,
            Self::Skipped(record) => &record.package,
        }
    }
}

/// JSON Schema for one survey JSONL record.
pub fn record_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(SurveyRecord)).unwrap_or(serde_json::Value::Null)
}

/// Key that decides whether a record for a package is already present.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ResumeKey {
    /// An analyzed pair.
    Analyzed {
        /// Package name.
        package: String,
        /// Older version.
        from: String,
        /// Newer version.
        to: String,
    },
    /// A skipped package.
    Skipped {
        /// Package name.
        package: String,
    },
}

/// Resume key of one completed record.
pub(crate) fn resume_key(record: &SurveyRecord) -> ResumeKey {
    match record {
        SurveyRecord::Analyzed(record) => ResumeKey::Analyzed {
            package: record.package.clone(),
            from: record.from.clone(),
            to: record.to.clone(),
        },
        SurveyRecord::Skipped(record) => ResumeKey::Skipped {
            package: record.package.clone(),
        },
    }
}

/// Packages already covered by records in the output file.
#[derive(Debug, Default)]
pub(crate) struct ResumeIndex {
    analyzed: BTreeSet<String>,
    skipped: BTreeSet<String>,
}

impl ResumeIndex {
    /// Add the key of one completed record.
    pub(crate) fn insert(&mut self, record: &SurveyRecord) {
        match resume_key(record) {
            ResumeKey::Analyzed { package, .. } => {
                self.analyzed.insert(package);
            }
            ResumeKey::Skipped { package } => {
                self.skipped.insert(package);
            }
        }
    }

    /// Whether a package needs no work. An analyzed pair or a skip record both
    /// finish a package, because a run analyzes at most one pair per package.
    pub(crate) fn is_package_done(&self, package: &str) -> bool {
        self.skipped.contains(package) || self.analyzed.contains(package)
    }
}

/// Existing records plus the file fixup needed before appending.
#[derive(Debug, Default)]
pub(crate) struct ResumeState {
    /// Packages already covered.
    pub(crate) index: ResumeIndex,
    /// Length to keep when the file ends in a partial line.
    pub(crate) truncate_to: Option<u64>,
    /// Whether the file holds a complete last line without its newline.
    pub(crate) needs_leading_newline: bool,
}

/// Read the existing output file and prepare it for appending.
///
/// A row is written with one `write_all` while the writer mutex is held, so a
/// killed run can leave at most a partial final line. That tail cannot be
/// resumed or reported, so it is dropped; the package it belongs to is retried
/// because no complete record covers it.
pub(crate) fn read_resume(path: &Path) -> Result<ResumeState, SurveyError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(ResumeState::default()),
        Err(error) => return Err(SurveyError::Io(error)),
    };
    if bytes.is_empty() {
        return Ok(ResumeState::default());
    }

    let mut state = ResumeState::default();
    let complete_end = match bytes.iter().rposition(|byte| *byte == b'\n') {
        Some(position) => position + 1,
        None => 0,
    };

    for (offset, line) in bytes[..complete_end]
        .split(|byte| *byte == b'\n')
        .enumerate()
    {
        if line.trim_ascii().is_empty() {
            continue;
        }
        state.index.insert(&decode_record(line, offset + 1)?);
    }

    // A complete record without its newline is kept; anything else after the
    // last newline is a torn line and is truncated.
    let tail = &bytes[complete_end..];
    if tail.trim_ascii().is_empty() {
        return Ok(state);
    }
    let tail_line = bytes[..complete_end]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1;
    match decode_record(tail, tail_line) {
        Ok(record) => {
            state.index.insert(&record);
            state.needs_leading_newline = true;
        }
        Err(SurveyError::Record { .. }) => {
            state.truncate_to = Some(complete_end as u64);
        }
        Err(error) => return Err(error),
    }
    Ok(state)
}

/// Read every record of a completed survey file.
pub(crate) fn read_records(path: &Path) -> Result<Vec<SurveyRecord>, SurveyError> {
    let bytes = fs::read(path)?;
    let mut records = Vec::new();
    for (offset, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.trim_ascii().is_empty() {
            continue;
        }
        records.push(decode_record(line, offset + 1)?);
    }
    Ok(records)
}

/// Decode one JSONL line and check the schema version.
fn decode_record(line: &[u8], line_number: usize) -> Result<SurveyRecord, SurveyError> {
    let record: SurveyRecord =
        serde_json::from_slice(line).map_err(|source| SurveyError::Record {
            line: line_number,
            source,
        })?;
    if record.schema() != RECORD_SCHEMA {
        return Err(SurveyError::Schema {
            line: line_number,
            schema: record.schema(),
            expected: RECORD_SCHEMA,
        });
    }
    Ok(record)
}

/// Append one record to the output file.
///
/// The record is serialized to one buffer before the write so a record reaches
/// the file in a single `write_all`, which keeps a killed run from leaving a
/// half-encoded line in the middle of the file.
pub(crate) fn append_record(
    writer: &mut impl std::io::Write,
    record: &SurveyRecord,
) -> Result<(), SurveyError> {
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    Ok(())
}

/// Create the output file and apply the resume fixups.
pub(crate) fn open_output(state: &ResumeState, path: &Path) -> Result<fs::File, SurveyError> {
    ensure_parent(path)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    if let Some(length) = state.truncate_to {
        file.set_len(length)?;
    }
    if state.needs_leading_newline {
        file.write_all(b"\n")?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an analyzed record with one counted rule.
    fn analyzed(package: &str, from: &str, to: &str) -> SurveyRecord {
        SurveyRecord::Analyzed(AnalyzedRecord {
            schema: RECORD_SCHEMA,
            package: package.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            classification: Classification::Breaking,
            observed_bump: Bump::Major,
            assessment: Assessment::Consistent,
            counts: Counts {
                breaking: 1,
                ..Counts::default()
            },
            rules: BTreeMap::from([("pvp/entity-removed".to_string(), 1)]),
            change_count: 1,
            unknown_count: 0,
            docs_sha256: DocsDigests {
                older: "aa".to_string(),
                newer: "bb".to_string(),
            },
            diagnostics: vec!["internal-modules-excluded".to_string()],
        })
    }

    /// Check that analyzed and skipped records serialize to the documented
    /// JSONL shape and decode back to the same value.
    #[test]
    fn serializes_both_record_kinds() {
        let analyzed = analyzed("demo", "1.0.0", "2.0.0");
        let value = serde_json::to_value(&analyzed).expect("analyzed JSON");
        assert_eq!(value["schema"], 1);
        assert_eq!(value["outcome"], "analyzed");
        assert_eq!(value["package"], "demo");
        assert_eq!(value["from"], "1.0.0");
        assert_eq!(value["to"], "2.0.0");
        assert_eq!(value["classification"], "breaking");
        assert_eq!(value["observedBump"], "major");
        assert_eq!(value["assessment"], "consistent");
        assert_eq!(value["counts"]["breaking"], 1);
        assert_eq!(value["rules"]["pvp/entity-removed"], 1);
        assert_eq!(value["changeCount"], 1);
        assert_eq!(value["unknownCount"], 0);
        assert_eq!(value["docsSha256"]["older"], "aa");
        assert_eq!(value["docsSha256"]["newer"], "bb");
        assert_eq!(value["diagnostics"][0], "internal-modules-excluded");
        assert_eq!(
            serde_json::from_value::<SurveyRecord>(value).expect("analyzed decode"),
            analyzed
        );

        let skipped = SurveyRecord::Skipped(SkippedRecord {
            schema: RECORD_SCHEMA,
            package: "demo".to_string(),
            reason: REASON_DOCS.to_string(),
            probed: vec!["2.0.0".to_string(), "1.0.0".to_string()],
        });
        let value = serde_json::to_value(&skipped).expect("skipped JSON");
        assert_eq!(value["schema"], 1);
        assert_eq!(value["outcome"], "skipped");
        assert_eq!(value["reason"], "docs");
        assert_eq!(value["probed"][0], "2.0.0");
        assert_eq!(
            serde_json::from_value::<SurveyRecord>(value).expect("skipped decode"),
            skipped
        );
        assert_eq!(skipped.schema(), 1);
        assert_eq!(skipped.package(), "demo");
    }

    /// Check that the schema names both outcomes and the analyzed fields, so a
    /// committed schema cannot drift from the struct shape.
    #[test]
    fn exposes_record_schema() {
        let schema = record_schema();
        assert!(schema.is_object());
        let text = schema.to_string();
        assert!(text.contains("\"analyzed\""));
        assert!(text.contains("\"skipped\""));
        assert!(text.contains("\"docsSha256\""));
        assert!(text.contains("\"observedBump\""));
        assert!(text.contains("\"probed\""));
        assert!(schema.get("oneOf").is_some());
    }

    /// Check the resume keys: an analyzed pair records its versions and a skip
    /// records only the package, and both finish the package.
    #[test]
    fn derives_resume_keys() {
        let analyzed = analyzed("demo", "1.0.0", "2.0.0");
        assert_eq!(
            resume_key(&analyzed),
            ResumeKey::Analyzed {
                package: "demo".to_string(),
                from: "1.0.0".to_string(),
                to: "2.0.0".to_string(),
            }
        );
        let skipped = SurveyRecord::Skipped(SkippedRecord {
            schema: RECORD_SCHEMA,
            package: "other".to_string(),
            reason: REASON_DOCS.to_string(),
            probed: Vec::new(),
        });
        assert_eq!(
            resume_key(&skipped),
            ResumeKey::Skipped {
                package: "other".to_string(),
            }
        );

        let mut index = ResumeIndex::default();
        index.insert(&analyzed);
        index.insert(&skipped);
        assert!(index.is_package_done("demo"));
        assert!(index.is_package_done("other"));
        assert!(!index.is_package_done("third"));
    }

    /// Check resume reading: complete lines count, a complete line without a
    /// newline is kept, and a torn last line is marked for truncation.
    #[test]
    fn reads_partial_output() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let path = workspace.path().join("survey.jsonl");
        let line = serde_json::to_string(&analyzed("demo", "1.0.0", "2.0.0")).expect("line");

        fs::write(&path, format!("{line}\n")).expect("write complete file");
        let state = read_resume(&path).expect("complete resume");
        assert_eq!(state.truncate_to, None);
        assert!(!state.needs_leading_newline);
        assert!(state.index.is_package_done("demo"));

        fs::write(&path, &line).expect("write file without newline");
        let state = read_resume(&path).expect("unterminated resume");
        assert_eq!(state.truncate_to, None);
        assert!(state.needs_leading_newline);
        assert!(state.index.is_package_done("demo"));

        let torn = format!("{line}\n{{\"schema\":1,\"outcome\":\"ana");
        fs::write(&path, &torn).expect("write torn file");
        let state = read_resume(&path).expect("torn resume");
        assert_eq!(state.truncate_to, Some((line.len() + 1) as u64));
        assert!(!state.needs_leading_newline);
        assert!(state.index.is_package_done("demo"));
    }

    /// Check that a record with a future schema version is rejected instead of
    /// being treated as a resume key.
    #[test]
    fn rejects_unknown_schema() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let path = workspace.path().join("survey.jsonl");
        let line = "{\"schema\":2,\"outcome\":\"skipped\",\"package\":\"demo\",\"reason\":\"docs\",\"probed\":[]}\n";
        fs::write(&path, line).expect("write file");
        assert!(matches!(
            read_records(&path),
            Err(SurveyError::Schema {
                line: 1,
                schema: 2,
                expected: RECORD_SCHEMA,
            })
        ));
    }
}
