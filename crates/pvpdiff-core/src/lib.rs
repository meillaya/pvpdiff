//! Core diff and classification layer for pvpdiff.
//!
//! This crate normalizes parsed APIs, compares two surfaces, classifies the
//! changes against the Package Versioning Policy, and renders the JSON, text,
//! Markdown, and SARIF reports. It depends on `pvpdiff-parse` and performs no
//! network access.

pub mod classify;
pub mod diff;
pub mod normalize;
pub mod report;
pub mod rules;
pub mod version;

pub use classify::{Classification, Confidence, Severity};
pub use diff::{
    Change, Counts, Diagnostic as DiffDiagnostic, DiffOptions, DiffResult, PackageApi, diff,
};
pub use normalize::{Aliases, Comparison, compare_signatures};
pub use report::{ArtifactInfo, InputInfo, PvpSummary, Report, ToolInfo, build_report};
pub use rules::{Rule, all as all_rules, rule};
pub use version::{
    Assessment, Bump, HaskellVersion, VersionError, assess, observed_bump, suggest,
    suggestion_rationale,
};

/// Version of this crate, taken from `Cargo.toml`.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
