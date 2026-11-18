//! Resumable survey of PVP classifications across Hackage.
//!
//! The survey walks the cached package index from `pvpdiff-index`, probes the
//! newest Hoogle databases for each package through `pvpdiff-hackage`, and
//! appends one JSONL record per package. A later run reads those records and
//! skips the packages they cover, so an interrupted survey loses only the
//! package in flight.
//!
//! Network access happens only in `pvpdiff-hackage`. A run with
//! [`SurveyOptions::offline`] set reads the cache and never opens a socket,
//! which keeps the test suite and a repeated dataset build offline.

mod record;
mod report;
mod selection;
mod survey;

use std::fs;
use std::path::Path;

use thiserror::Error;

pub use record::{
    AnalyzedRecord, DocsDigests, RECORD_SCHEMA, SkippedRecord, SurveyRecord, record_schema,
};
pub use report::report;
pub use survey::{SurveyOptions, SurveySummary, run};

/// Failure to run or report a survey.
#[derive(Debug, Error)]
pub enum SurveyError {
    /// The cached index database could not be read.
    #[error("index error: {0}")]
    Index(#[from] pvpdiff_index::IndexError),
    /// The Hackage client could not be built.
    #[error("Hackage error: {0}")]
    Hackage(#[from] pvpdiff_hackage::HackageError),
    /// A local file operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A JSON value could not be encoded or decoded.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// One JSONL line is not a survey record.
    #[error("invalid survey record at line {line}: {source}")]
    Record {
        /// One based line number.
        line: usize,
        /// Decode error.
        source: serde_json::Error,
    },
    /// One JSONL line carries an unsupported schema version.
    #[error("survey record at line {line} has schema {schema}, expected {expected}")]
    Schema {
        /// One based line number.
        line: usize,
        /// Schema version found in the record.
        schema: u32,
        /// Schema version this crate writes.
        expected: u32,
    },
    /// The shard tuple cannot divide the candidate packages.
    #[error(
        "invalid shard {index}/{count}: the index must be below the count and the count must be positive"
    )]
    InvalidShard {
        /// Shard index.
        index: usize,
        /// Shard count.
        count: usize,
    },
}

/// Create the parent directory of `path` when the path names one.
pub(crate) fn ensure_parent(path: &Path) -> Result<(), SurveyError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Version of this crate, taken from `Cargo.toml`.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
