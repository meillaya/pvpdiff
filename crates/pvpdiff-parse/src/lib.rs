//! Parsing layer for pvpdiff.
//!
//! This crate owns the normalized model of a package API and the two parsers
//! that produce it: the Hackage Hoogle text-database parser and the Cabal file
//! adapter. It performs no network access and knows nothing about the PVP.
//!
//! The implementation is filled in by phase P1; P0 only establishes the
//! workspace boundary so the Nix/devenv scaffold can be verified end to end.

/// Version of this crate, taken from `Cargo.toml`.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
