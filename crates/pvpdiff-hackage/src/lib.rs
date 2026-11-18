//! Hackage access layer for pvpdiff.
//!
//! This crate owns the URL layout, the blocking HTTP client, the ETag/content
//! cache, and the streaming reader for Hackage's `01-index.tar.gz`. It returns
//! bytes and files and deliberately does not parse Hoogle databases or Cabal
//! files itself.

pub mod cache;
pub mod client;
pub mod urls;

pub use cache::{Artifact, ArtifactKind, Cache, CacheError, CacheHeaders, CacheMeta, sha256_hex};
pub use client::{HackageClient, HackageError};
pub use urls::DEFAULT_BASE_URL;

/// User agent sent with every Hackage request.
pub fn user_agent() -> String {
    format!(
        "pvpdiff/{} (+https://github.com/meillaya/pvpdiff)",
        env!("CARGO_PKG_VERSION")
    )
}
