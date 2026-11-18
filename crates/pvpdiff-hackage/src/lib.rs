//! Hackage access layer for pvpdiff.
//!
//! This crate owns the URL layout, the blocking HTTP client, the ETag/content
//! cache, and the streaming reader for Hackage's `01-index.tar.gz`. It returns
//! bytes and files and deliberately does not parse Hoogle databases or Cabal
//! files itself.
//!
//! The implementation is filled in by phase P1; P0 only establishes the
//! workspace boundary so the Nix/devenv scaffold can be verified end to end.

/// User agent sent with every Hackage request.
pub fn user_agent() -> String {
    format!(
        "pvpdiff/{} (+https://github.com/meillaya/pvpdiff)",
        env!("CARGO_PKG_VERSION")
    )
}
