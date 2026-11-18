//! Hackage index synchronization and reverse-dependency blast radius.

/// Version of this crate, taken from `Cargo.toml`.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
