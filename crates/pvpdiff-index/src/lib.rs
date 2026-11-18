//! Hackage index synchronization, reverse-dependency blast radius, and Stackage
//! snapshot pins for pvpdiff.
//!
//! `sync_index` streams `01-index.tar.gz` into a JSONL dependency database,
//! `blast_radius` evaluates the dependents' Cabal bounds against a proposed
//! version, and `fetch_snapshot` reads a Stackage `cabal.config` pin set. Only
//! those functions touch the network; every test runs from a local fixture.

pub mod blast;
pub mod index;
pub mod stackage;

pub use blast::{BlastCounts, BlastDependent, BlastReport, BlastSource, BlastStatus, blast_radius};
pub use index::{
    DEFAULT_INDEX_URL, IndexDependency, IndexError, IndexMeta, IndexRow, read_index, read_meta,
    sha256_hex, sync_index,
};
pub use stackage::{SnapshotPins, fetch_snapshot, parse_snapshot, snapshot_url};

/// User agent sent with Hackage and Stackage requests.
pub(crate) fn user_agent() -> String {
    format!(
        "pvpdiff/{} (+https://github.com/meillaya/pvpdiff)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Version of this crate, taken from `Cargo.toml`.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
