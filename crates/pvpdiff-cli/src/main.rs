//! Command-line entry point for pvpdiff.
//!
//! The CLI parses arguments, loads Hackage or local inputs, calls the core diff
//! and report layers, and maps outcomes to exit codes. Business logic lives in
//! the library crates.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use pvpdiff_core::diff::{Diagnostic, DiffOptions};
use pvpdiff_core::report::{ArtifactInfo, Report, SideArtifacts, build_report, unavailable_report};
use pvpdiff_core::{Classification, PackageApi, all_rules, diff};
use pvpdiff_hackage::{HackageClient, HackageError, sha256_hex};
use pvpdiff_parse::{ApiSurface, parse_cabal, parse_hoogle_bytes};

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

#[allow(clippy::large_enum_variant)]
// Clap subcommands carry their own arguments; boxing just moves the size to
// the heap and complicates the derive.
#[derive(Debug, Subcommand)]
enum Command {
    /// Compare two Hackage versions or two local Hoogle databases.
    Diff(DiffArgs),
    /// Compare a local working tree with a published Hackage version.
    Check(CheckArgs),
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
    #[command(flatten)]
    output: OutputArgs,
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

#[derive(Debug, Args)]
struct OutputArgs {
    /// Output format written to stdout.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
    /// Write the JSON report to this file as well.
    #[arg(long = "json-out")]
    json_out: Option<PathBuf>,
    /// Write SARIF 2.1.0 to this file.
    #[arg(long)]
    sarif: Option<PathBuf>,
    /// Repository-relative path embedded in SARIF locations.
    #[arg(long = "sarif-uri")]
    sarif_uri: Option<String>,
    /// Include internal-module results in SARIF.
    #[arg(long = "sarif-all")]
    sarif_all: bool,
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// Directory containing the package `.cabal` file.
    #[arg(long = "package-dir", default_value = ".")]
    package_dir: PathBuf,
    /// Version to compare against: `latest`, `previous`, or an explicit version.
    #[arg(long, default_value = "latest")]
    against: String,
    /// Use an existing Hoogle database instead of running `cabal haddock`.
    #[arg(long)]
    docs: Option<PathBuf>,
    /// Do not build docs; requires `--docs`.
    #[arg(long = "no-build")]
    no_build: bool,
    /// Build directory for the local Haddock run.
    #[arg(long = "build-dir")]
    build_dir: Option<PathBuf>,
    #[command(flatten)]
    output: OutputArgs,
    /// Fail with exit code 100 when this condition is met.
    #[arg(long, value_enum, default_value_t = FailOn::Never)]
    fail_on: FailOn,
    /// Apply the PVP-literal policy.
    #[arg(long)]
    strict: bool,
    /// Count changes in Internal/Private modules.
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
        Command::Check(args) => run_check(args),
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
        emit_report(&report, &args.output)?;
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
    emit_report(&report, &args.output)?;
    Ok(gate_exit(&report, args.fail_on))
}

fn emit_report(report: &Report, args: &OutputArgs) -> Result<()> {
    if let Some(path) = &args.json_out {
        fs::write(path, report.to_json()?)
            .with_context(|| format!("write JSON report to {}", path.display()))?;
    }
    if let Some(path) = &args.sarif {
        let uri = args
            .sarif_uri
            .clone()
            .unwrap_or_else(|| format!("{}.cabal", report.package));
        let document = pvpdiff_core::sarif::render(report, &uri, args.sarif_all)?;
        fs::write(path, document)
            .with_context(|| format!("write SARIF report to {}", path.display()))?;
    }
    match args.format {
        OutputFormat::Text => write_stdout(&report.to_text())?,
        OutputFormat::Json => write_stdout(&report.to_json()?)?,
        OutputFormat::Markdown => write_stdout(&report.to_markdown())?,
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

fn run_check(args: CheckArgs) -> Result<u8> {
    let package_dir = args
        .package_dir
        .canonicalize()
        .with_context(|| format!("resolve package directory {}", args.package_dir.display()))?;
    let cabal_path = find_cabal_file(&package_dir)?;
    let cabal_text = fs::read_to_string(&cabal_path)
        .with_context(|| format!("read Cabal file {}", cabal_path.display()))?;
    let parsed_cabal = parse_cabal(&cabal_text);
    let cabal_meta = parsed_cabal
        .meta
        .clone()
        .context("the local Cabal file has no usable package name and version; cannot run check")?;
    let package = cabal_meta.name.clone();
    let local_version = cabal_meta.version.clone();

    let cache = cache_dir(args.cache_dir.as_ref())?;
    let client = HackageClient::new(cache.clone(), args.offline, args.refresh)?;

    let cabal_bytes = fs::read(&cabal_path)
        .with_context(|| format!("read Cabal file {}", cabal_path.display()))?;
    let local = load_local_check_side(
        &args,
        &package_dir,
        &cabal_path,
        &cabal_bytes,
        &cache,
        cabal_meta,
        parsed_cabal,
    )?;
    let baseline = resolve_baseline(&client, &package, &local_version, &args.against)?;

    if local.api.surface.package != baseline.api.surface.package {
        bail!(
            "the local package {} does not match the Hackage baseline {}",
            local.api.surface.package,
            baseline.api.surface.package
        );
    }
    if local.api.surface.version == baseline.api.surface.version {
        bail!(
            "local version {} equals the baseline version; bump the version before checking",
            local.api.surface.version
        );
    }

    let result = diff(
        &baseline.api,
        &local.api,
        DiffOptions {
            strict: args.strict,
            include_internal: args.include_internal,
        },
    );
    let mut diagnostics = baseline.diagnostics.clone();
    diagnostics.extend(local.diagnostics.clone());
    let report = build_report(
        &baseline.api,
        SideArtifacts {
            docs: baseline.docs,
            cabal: baseline.cabal,
        },
        &local.api,
        SideArtifacts {
            docs: local.docs,
            cabal: local.cabal,
        },
        &result,
        diagnostics,
    );
    emit_report(&report, &args.output)?;
    Ok(gate_exit(&report, args.fail_on))
}

fn find_cabal_file(package_dir: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(package_dir)
        .with_context(|| format!("list package directory {}", package_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "cabal")
        {
            candidates.push(path);
        }
    }
    candidates.sort();
    match candidates.len() {
        0 => bail!("no .cabal file found in {}", package_dir.display()),
        1 => Ok(candidates.remove(0)),
        _ => bail!(
            "multiple .cabal files found in {}; point --package-dir at one package",
            package_dir.display()
        ),
    }
}

fn load_local_check_side(
    args: &CheckArgs,
    package_dir: &Path,
    cabal_path: &Path,
    cabal_bytes: &[u8],
    cache: &Path,
    cabal_meta: pvpdiff_parse::CabalMeta,
    parsed_cabal: pvpdiff_parse::ParsedCabal,
) -> Result<LoadedSide> {
    let mut diagnostics = cabal_diagnostics(&parsed_cabal.diagnostics);
    let (docs_path, docs_bytes, surface) = if let Some(path) = args.docs.as_deref() {
        let bytes =
            fs::read(path).with_context(|| format!("read Hoogle database {}", path.display()))?;
        let surface = parse_hoogle_bytes(&bytes)
            .with_context(|| format!("parse Hoogle database {}", path.display()))?;
        (path.to_path_buf(), bytes, surface)
    } else {
        if args.no_build {
            bail!("--no-build requires --docs");
        }
        let build_dir = args
            .build_dir
            .clone()
            .unwrap_or_else(|| cache.join("haddock"));
        let build_dir = if build_dir.is_absolute() {
            build_dir
        } else {
            package_dir.join(build_dir)
        };
        fs::create_dir_all(&build_dir)
            .with_context(|| format!("create build directory {}", build_dir.display()))?;
        eprintln!(
            "pvpdiff: running cabal haddock in {} (build dir {})",
            package_dir.display(),
            build_dir.display()
        );
        let status = StdCommand::new("cabal")
            .current_dir(package_dir)
            .arg("haddock")
            .arg("--haddock-hoogle")
            .arg("--builddir")
            .arg(&build_dir)
            .status()
            .context("failed to run cabal; install GHC and cabal or pass --docs")?;
        if !status.success() {
            bail!("cabal haddock failed with {status}");
        }
        let (path, bytes, surface) =
            discover_local_docs(&build_dir, &cabal_meta.name, &cabal_meta.version)?;
        (path, bytes, surface)
    };
    diagnostics.extend(parser_diagnostics(&surface.warnings));
    if surface.package != cabal_meta.name || surface.version != cabal_meta.version {
        diagnostics.push(Diagnostic {
            code: "version-mismatch".to_string(),
            severity: pvpdiff_core::Severity::Warning,
            message: format!(
                "local Hoogle database says {}-{}, Cabal file says {}-{}",
                surface.package, surface.version, cabal_meta.name, cabal_meta.version
            ),
        });
    }
    Ok(LoadedSide {
        api: PackageApi {
            surface,
            cabal: Some(cabal_meta),
        },
        docs: Some(ArtifactInfo {
            url: format!("file:{}", docs_path.display()),
            sha256: sha256_hex(&docs_bytes),
        }),
        cabal: Some(ArtifactInfo {
            url: format!("file:{}", cabal_path.display()),
            sha256: sha256_hex(cabal_bytes),
        }),
        diagnostics,
    })
}

fn discover_local_docs(
    build_dir: &Path,
    package: &str,
    version: &str,
) -> Result<(PathBuf, Vec<u8>, ApiSurface)> {
    let mut candidates = Vec::new();
    collect_txt_files(build_dir, &mut candidates);
    candidates.sort();
    let mut found = Vec::new();
    for path in candidates {
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(surface) = parse_hoogle_bytes(&bytes) else {
            continue;
        };
        if surface.package == package && surface.version == version {
            found.push((path, bytes, surface));
        }
    }
    match found.len() {
        0 => bail!(
            "no Hoogle database for {package}-{version} under {}; check the cabal haddock output",
            build_dir.display()
        ),
        1 => Ok(found.remove(0)),
        _ => {
            eprintln!("pvpdiff: multiple local Hoogle databases matched; using the first");
            Ok(found.remove(0))
        }
    }
}

fn collect_txt_files(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_txt_files(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "txt") {
            output.push(path);
        }
    }
}

fn resolve_baseline(
    client: &HackageClient,
    package: &str,
    local_version: &str,
    against: &str,
) -> Result<LoadedSide> {
    let candidates: Vec<String> = match against {
        "latest" | "previous" => {
            if client.is_offline() {
                bail!("--offline cannot resolve `{against}`; pass an explicit baseline version");
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
            parsed.reverse();
            if against == "previous" && !parsed.is_empty() {
                parsed.remove(0);
            }
            parsed.into_iter().map(|(_, version)| version).collect()
        }
        explicit => vec![explicit.to_string()],
    };
    for candidate in candidates {
        if candidate == local_version {
            continue;
        }
        match load_hackage_side(client, package, &candidate)? {
            Some(side) => return Ok(side),
            None => continue,
        }
    }
    bail!("no Hackage Hoogle database with docs found for {package} ({against})")
}

fn write_stdout(text: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(text.as_bytes()) {
        Ok(()) => Ok(()),
        // A consumer such as `head` closing the pipe is not a tool failure.
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn run_schema(args: SchemaArgs) -> Result<u8> {
    let value = match args.name {
        SchemaName::Report => Report::schema_value(),
    };
    let mut text = serde_json::to_string_pretty(&value)?;
    text.push('\n');
    write_stdout(&text)?;
    Ok(0)
}

fn run_rules(args: RulesArgs) -> Result<u8> {
    let mut text = String::new();
    match args.format {
        OutputFormat::Text => {
            for rule in all_rules() {
                writeln!(&mut text, "{}: {}", rule.id, rule.summary)?;
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
            text = serde_json::to_string_pretty(&rules)?;
            text.push('\n');
        }
        OutputFormat::Markdown => {
            for rule in all_rules() {
                writeln!(&mut text, "- `{}`: {}", rule.id, rule.summary)?;
            }
        }
    }
    write_stdout(&text)?;
    Ok(0)
}

fn run_cache(args: CacheArgs) -> Result<u8> {
    match args.command {
        CacheCommand::Dir {
            cache_dir: directory,
        } => {
            write_stdout(&format!("{}\n", cache_dir(directory.as_ref())?.display()))?;
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
