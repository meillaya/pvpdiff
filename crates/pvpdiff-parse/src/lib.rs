//! Parsing layer for pvpdiff.
//!
//! This crate owns the normalized model of a package API and the two parsers
//! that produce it: the Hackage Hoogle text-database parser and the Cabal file
//! adapter. It performs no network access and knows nothing about the PVP.

pub mod cabal;
pub mod hoogle;
pub mod model;

pub use cabal::parse_cabal;
pub use hoogle::{HoogleParseError, parse_hoogle, parse_hoogle_bytes};
pub use model::{
    ApiSurface, Associativity, CabalMeta, ComponentKind, DependencyMeta, Entity, EntityKey,
    EntityKind, Fixity, Instance, LibraryMeta, ModuleApi, ParsedCabal,
};

/// Version of this crate, taken from `Cargo.toml`.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
