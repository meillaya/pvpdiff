//! Command-line entry point for pvpdiff.
//!
//! The CLI parses arguments, loads Hackage or local inputs, calls the core diff
//! and report layers, and maps outcomes to exit codes. Business logic lives in
//! the library crates.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use pvpdiff_core::diff::{Diagnostic, DiffOptions};
use pvpdiff_core::report::{ArtifactInfo, Report, SideArtifacts, build_report, unavailable_report};
use pvpdiff_core::{Classification, PackageApi, all_rules, diff};
use pvpdiff_hackage::{HackageClient, HackageError, sha256_hex};
use pvpdiff_parse::{parse_cabal, parse_hoogle_bytes};

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
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compare two Hackage versions or two local Hoogle databases.
    Diff(DiffArgs),
    /// Print a committed JSON Schema.
    Schema(SchemaArgs),
    /// Print the stable rule catalog.
    Rules(RulesArgs),
    /// Inspect or clear the local cache.
    Cache(CacheArgs),
}

#[derive(Debug, Args)]
struct DiffArgs {
    /// Hackage package name for a Hackage-to-Hackage diff.
    #[arg(long)]
    package: Option<String>,
    /// Old version, or `latest`/`previous`.
    #[arg(long, visible_alias = "old")]
    from: Option<String>,
    /// New version, or `latest`/`previous`.
    #[arg(long, visible_alias = "new")]
    to: Option<String>,
    /// Old Hoogle database for a local diff.
    #[arg(long = "from-docs")]
    from_docs: Option<PathBuf>,
    /// New Hoogle database for a local diff.
    #[arg(long = "to-docs")]
    to_docs: Option<PathBuf>,
    /// Old Cabal file for a local diff.
    #[arg(long = "from-cabal")]
    from_cabal: Option<PathBuf>,
    /// New Cabal file for a local diff.
    #[arg(long = "to-cabal")]
    to_cabal: Option<PathBuf>,
    /// Output format written to stdout.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
    /// Write the JSON report to this file as well.
    #[arg(long = "json-out")]
    json_out: Option<PathBuf>,
    /// Fail with exit code 100 when this condition is met.
    #[arg(long, value_enum, default_value_t = FailOn::Never)]
    fail_on: FailOn,
    /// Apply the PVP-literal policy for internal modules, instances,
    /// deprecations, fixities, and added class methods.
    #[arg(long)]
    strict: bool,
    /// Count changes in Internal/Private modules without changing the other
    /// conservative rules.
    #[arg(long = "include-internal")]
    include_internal: bool,
    /// Override the cache directory.
    #[arg(long = "cache-dir")]
    cache_dir: Option<PathBuf>,
    /// Use only cached artifacts.
    #[arg(long)]
    offline: bool,
    /// Ignore cache freshness and refetch mutable artifacts.
    #[arg(long)]
    refresh: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    /// Human-readable text.
    Text,
    /// JSON report.
    Json,
    /// Markdown changelog skeleton.
    Markdown,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FailOn {
    /// Never fail on findings.
    Never,
    /// Fail on a breaking-change candidate.
    Breaking,
    /// Fail on an unknown classification.
    Unknown,
    /// Fail on any non-empty change set.
    Any,
}

#[derive(Debug, Args)]
struct SchemaArgs {
    /// Schema name.
    #[arg(value_enum, default_value_t = SchemaName::Report)]
    name: SchemaName,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SchemaName {
    /// The diff report.
    Report,
}

#[derive(Debug, Args)]
struct RulesArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

#[derive(Debug, Args)]
struct CacheArgs {
    #[command(subcommand)]
    command: CacheCommand,
}

#[derive(Debug, Subcommand)]
enum CacheCommand {
    /// Print the cache directory.
    Dir {
        /// Override the cache directory.
        #[arg(long = "cache-dir")]
        cache_dir: Option<PathBuf>,
    },
    /// Remove cached artifacts.
    Clean {
        /// Override the cache directory.
        #[arg(long = "cache-dir")]
        cache_dir: Option<PathBuf>,
    },
}

struct LoadedSide {
    api: PackageApi,
    docs: Option<ArtifactInfo>,
    cabal: Option<ArtifactInfo>,
    diagnostics: Vec<Diagnostic>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("pvpdiff: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    match cli.command {
        Command::Diff(args) => run_diff(args),
        Command::Schema(args) => run_schema(args),
        Command::Rules(args) => run_rules(args),
        Command::Cache(args) => run_cache(args),
    }
}

fn run_diff(args: DiffArgs) -> Result<u8> {
    if args.from_docs.is_some() || args.to_docs.is_some() {
        return run_local_diff(args);
    }
    run_hackage_diff(args)
}

fn run_hackage_diff(args: DiffArgs) -> Result<u8> {
    let Some(package) = args.package.as_deref() else {
        bail!("--package is required for a Hackage diff");
    };
    let Some(from_request) = args.from.as_deref() else {
        bail!("--from is required for a Hackage diff");
    };
    let Some(to_request) = args.to.as_deref() else {
        bail!("--to is required for a Hackage diff");
    };
    let cache_dir = cache_dir(args.cache_dir.as_ref())?;
    let client = HackageClient::new(cache_dir, args.offline, args.refresh)?;

    let from_version = resolve_version(&client, package, from_request)?;
    let to_version = resolve_version(&client, package, to_request)?;

    let old = load_hackage_side(&client, package, &from_version)?;
    let new = load_hackage_side(&client, package, &to_version)?;

    let (Some(old), Some(new)) = (old, new) else {
        let diagnostics = vec![Diagnostic {
            code: "docs-unavailable".to_string(),
            severity: pvpdiff_core::Severity::Warning,
            message: format!(
                "a Hoogle database for {package} ({from_version} -> {to_version}) is unavailable"
            ),
        }];
        let report = unavailable_report(package, &from_version, &to_version, diagnostics);
        emit_report(&report, &args)?;
        return Ok(gate_exit(&report, args.fail_on));
    };

    finish_diff(old, new, args)
}

fn run_local_diff(args: DiffArgs) -> Result<u8> {
    if args.package.is_some() || args.from.is_some() || args.to.is_some() {
        bail!("use either --package/--from/--to or --from-docs/--to-docs");
    }
    let Some(from_docs) = args.from_docs.as_deref() else {
        bail!("--from-docs is required for a local diff");
    };
    let Some(to_docs) = args.to_docs.as_deref() else {
        bail!("--to-docs is required for a local diff");
    };
    let old = load_local_side(from_docs, args.from_cabal.as_deref())?;
    let new = load_local_side(to_docs, args.to_cabal.as_deref())?;
    if old.api.surface.package != new.api.surface.package {
        bail!(
            "the two databases describe different packages: {} and {}",
            old.api.surface.package,
            new.api.surface.package
        );
    }
    finish_diff(old, new, args)
}

fn finish_diff(old: LoadedSide, new: LoadedSide, args: DiffArgs) -> Result<u8> {
    let options = DiffOptions {
        strict: args.strict,
        include_internal: args.include_internal,
    };
    let mut diagnostics = old.diagnostics.clone();
    diagnostics.extend(new.diagnostics.clone());
    let result = diff(&old.api, &new.api, options);
    let report = build_report(
        &old.api,
        SideArtifacts {
            docs: old.docs,
            cabal: old.cabal,
        },
        &new.api,
        SideArtifacts {
            docs: new.docs,
            cabal: new.cabal,
        },
        &result,
        diagnostics,
    );
    emit_report(&report, &args)?;
    Ok(gate_exit(&report, args.fail_on))
}

fn emit_report(report: &Report, args: &DiffArgs) -> Result<()> {
    if let Some(path) = &args.json_out {
        fs::write(path, report.to_json()?)
            .with_context(|| format!("write JSON report to {}", path.display()))?;
    }
    match args.format {
        OutputFormat::Text => print!("{}", report.to_text()),
        OutputFormat::Json => print!("{}", report.to_json()?),
        OutputFormat::Markdown => print!("{}", report.to_markdown()),
    }
    Ok(())
}

fn gate_exit(report: &Report, fail_on: FailOn) -> u8 {
    let triggered = match fail_on {
        FailOn::Never => false,
        FailOn::Breaking => report.classification == Classification::Breaking,
        FailOn::Unknown => report.classification == Classification::Unknown,
        FailOn::Any => report.classification != Classification::None,
    };
    if triggered {
        eprintln!(
            "pvpdiff: --fail-on triggered by a {} classification",
            report.classification.as_str()
        );
        100
    } else {
        0
    }
}

fn load_hackage_side(
    client: &HackageClient,
    package: &str,
    version: &str,
) -> Result<Option<LoadedSide>> {
    let docs = match client.fetch_docs(package, version) {
        Ok(docs) => docs,
        Err(HackageError::DocsUnavailable { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let surface = parse_hoogle_bytes(&docs.bytes)
        .with_context(|| format!("parse Hoogle database for {package}-{version}"))?;
    let mut diagnostics = parser_diagnostics(&surface.warnings);
    if surface.package != package || surface.version != version {
        diagnostics.push(Diagnostic {
            code: "version-mismatch".to_string(),
            severity: pvpdiff_core::Severity::Warning,
            message: format!(
                "Hoogle database says {}-{}, expected {package}-{version}",
                surface.package, surface.version
            ),
        });
    }

    let mut cabal = None;
    let mut cabal_artifact = None;
    if let Some(artifact) = client.fetch_cabal(package, version)? {
        let text = std::str::from_utf8(&artifact.bytes)
            .with_context(|| format!("decode Cabal file for {package}-{version}"))?;
        let parsed = parse_cabal(text);
        diagnostics.extend(cabal_diagnostics(&parsed.diagnostics));
        cabal = parsed.meta;
        cabal_artifact = Some(ArtifactInfo {
            url: artifact.url,
            sha256: artifact.sha256,
        });
    }

    Ok(Some(LoadedSide {
        api: PackageApi { surface, cabal },
        docs: Some(ArtifactInfo {
            url: docs.url,
            sha256: docs.sha256,
        }),
        cabal: cabal_artifact,
        diagnostics,
    }))
}

fn load_local_side(docs_path: &Path, cabal_path: Option<&Path>) -> Result<LoadedSide> {
    let bytes = fs::read(docs_path)
        .with_context(|| format!("read Hoogle database {}", docs_path.display()))?;
    let surface = parse_hoogle_bytes(&bytes)
        .with_context(|| format!("parse Hoogle database {}", docs_path.display()))?;
    let mut diagnostics = parser_diagnostics(&surface.warnings);
    let mut cabal = None;
    let mut cabal_artifact = None;
    if let Some(cabal_path) = cabal_path {
        let cabal_bytes = fs::read(cabal_path)
            .with_context(|| format!("read Cabal file {}", cabal_path.display()))?;
        let text = std::str::from_utf8(&cabal_bytes)
            .with_context(|| format!("decode Cabal file {}", cabal_path.display()))?;
        let parsed = parse_cabal(text);
        diagnostics.extend(cabal_diagnostics(&parsed.diagnostics));
        cabal = parsed.meta;
        cabal_artifact = Some(ArtifactInfo {
            url: format!("file:{}", cabal_path.display()),
            sha256: sha256_hex(&cabal_bytes),
        });
    }
    Ok(LoadedSide {
        api: PackageApi { surface, cabal },
        docs: Some(ArtifactInfo {
            url: format!("file:{}", docs_path.display()),
            sha256: sha256_hex(&bytes),
        }),
        cabal: cabal_artifact,
        diagnostics,
    })
}

fn parser_diagnostics(warnings: &[String]) -> Vec<Diagnostic> {
    warnings
        .iter()
        .map(|warning| Diagnostic {
            code: "hoogle-parse-warning".to_string(),
            severity: pvpdiff_core::Severity::Info,
            message: warning.clone(),
        })
        .collect()
}

fn cabal_diagnostics(diagnostics: &[String]) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .map(|diagnostic| Diagnostic {
            code: "cabal-parse".to_string(),
            severity: pvpdiff_core::Severity::Warning,
            message: diagnostic.clone(),
        })
        .collect()
}

fn resolve_version(client: &HackageClient, package: &str, requested: &str) -> Result<String> {
    match requested {
        "latest" | "previous" => {
            if client.is_offline() {
                bail!("--offline cannot resolve `{requested}`; pass an explicit version");
            }
            let versions = client.versions(package)?;
            let mut parsed: Vec<(pvpdiff_core::HaskellVersion, String)> = versions
                .keys()
                .filter_map(|version| {
                    pvpdiff_core::HaskellVersion::parse(version)
                        .ok()
                        .map(|parsed| (parsed, version.clone()))
                })
                .collect();
            parsed.sort();
            if parsed.is_empty() {
                bail!("package {package} has no parseable versions");
            }
            if requested == "latest" {
                return Ok(parsed.last().expect("non-empty").1.clone());
            }
            if parsed.len() < 2 {
                bail!("package {package} has only one version");
            }
            Ok(parsed[parsed.len() - 2].1.clone())
        }
        explicit => Ok(explicit.to_string()),
    }
}

fn cache_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(directory) = override_dir {
        return Ok(directory.clone());
    }
    if let Ok(directory) = std::env::var("PVPDIFF_CACHE_DIR") {
        return Ok(PathBuf::from(directory));
    }
    Ok(HackageClient::default_cache_dir()?)
}

fn run_schema(args: SchemaArgs) -> Result<u8> {
    let value = match args.name {
        SchemaName::Report => Report::schema_value(),
    };
    let mut text = serde_json::to_string_pretty(&value)?;
    text.push('\n');
    print!("{text}");
    Ok(0)
}

fn run_rules(args: RulesArgs) -> Result<u8> {
    match args.format {
        OutputFormat::Text => {
            for rule in all_rules() {
                println!("{}: {}", rule.id, rule.summary);
            }
        }
        OutputFormat::Json => {
            let rules: Vec<serde_json::Value> = all_rules()
                .iter()
                .map(|rule| {
                    serde_json::json!({
                        "id": rule.id,
                        "summary": rule.summary,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rules)?);
        }
        OutputFormat::Markdown => {
            for rule in all_rules() {
                println!("- `{}`: {}", rule.id, rule.summary);
            }
        }
    }
    Ok(0)
}

fn run_cache(args: CacheArgs) -> Result<u8> {
    match args.command {
        CacheCommand::Dir {
            cache_dir: directory,
        } => {
            println!("{}", cache_dir(directory.as_ref())?.display());
        }
        CacheCommand::Clean {
            cache_dir: directory,
        } => {
            let directory = cache_dir(directory.as_ref())?;
            let client = HackageClient::new(directory.clone(), false, false)?;
            client.clear_cache()?;
            eprintln!("pvpdiff: cleared {}", directory.display());
        }
    }
    Ok(0)
}
