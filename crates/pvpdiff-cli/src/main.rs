//! Command-line entry point for pvpdiff.
//!
//! The CLI is intentionally thin: it parses arguments, calls into
//! `pvpdiff-core` and `pvpdiff-hackage`, formats output, and maps outcomes to
//! exit codes. Business logic lives in the library crates.
//!
//! P0 ships only the binary skeleton and the version/help surface. The
//! subcommands are added incrementally in P1.

use clap::Parser;

/// PVP-aware public API diff for Hackage packages.
#[derive(Debug, Parser)]
#[command(
    name = "pvpdiff",
    version,
    about = "PVP-aware public API diff for Hackage packages",
    long_about = "Compare two versions of a Haskell package using Hackage's \
                  per-version Hoogle databases and classify the changes against \
                  the Package Versioning Policy."
)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
