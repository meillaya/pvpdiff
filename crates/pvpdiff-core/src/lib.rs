//! Core diff and classification layer for pvpdiff.
//!
//! This crate normalizes parsed APIs, compares two surfaces, classifies the
//! changes against the Package Versioning Policy, and renders the JSON, text,
//! Markdown, and SARIF reports. It depends on `pvpdiff-parse` and performs no
//! network access.
//!
//! The implementation is filled in by phase P1; P0 only establishes the
//! workspace boundary so the Nix/devenv scaffold can be verified end to end.

/// Version of this crate, taken from `Cargo.toml`.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
