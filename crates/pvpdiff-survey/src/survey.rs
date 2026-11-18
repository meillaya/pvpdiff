//! The survey run: probing, analysis, concurrency, and resume.
//!
//! Each candidate package is handled independently. A worker probes the newest
//! versions for Hoogle databases, builds one record from the newest two that
//! exist, and appends the record to the shared output file. The output file is
//! guarded by a mutex so records do not interleave; the network pacing runs per
//! worker, so `jobs` workers issue at most one fetch per `delay` each.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, sleep};
use std::time::Duration;

use pvpdiff_core::{Assessment, DiffOptions, DiffResult, PackageApi, assess, diff, observed_bump};
use pvpdiff_hackage::{HackageClient, HackageError};
use pvpdiff_index::read_index;
use pvpdiff_parse::parse_hoogle_bytes;

use crate::SurveyError;
use crate::record::{
    AnalyzedRecord, DocsDigests, REASON_DOCS, REASON_FETCH, REASON_PARSE, RECORD_SCHEMA,
    SkippedRecord, SurveyRecord, append_record, open_output, read_resume,
};
use crate::selection::{CandidateVersion, PackageCandidate, select_candidates};

/// Number of Hoogle databases a pair needs.
const DOCS_NEEDED: usize = 2;
/// Progress lines are written to stderr every this many completed packages.
const PROGRESS_EVERY: usize = 100;

/// Options for one survey run.
#[derive(Debug, Clone)]
pub struct SurveyOptions {
    /// Root cache directory holding the synced index and the Hackage artifacts.
    pub cache_dir: PathBuf,
    /// Append-only JSONL file for the survey records.
    pub output: PathBuf,
    /// Maximum number of candidate packages to select.
    pub limit: Option<usize>,
    /// Worker threads. Zero and one run sequentially.
    pub jobs: usize,
    /// Pause between Hackage fetches, measured per worker.
    pub delay: Duration,
    /// Use only cached artifacts and never touch the network.
    pub offline: bool,
    /// Refetch mutable artifacts even when the cached copy is fresh.
    pub refresh: bool,
    /// Restrict the run to one shard of the package-name hash. The tuple is
    /// `(index, count)` with `index < count`.
    pub shard: Option<(usize, usize)>,
    /// Number of newest versions to probe for a Hoogle database.
    pub max_versions: usize,
}

/// Outcome counts of one survey run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurveySummary {
    /// Candidate packages selected by the run after the shard and limit.
    pub packages: usize,
    /// Pairs analyzed and appended by this run.
    pub analyzed: usize,
    /// Packages skipped and appended by this run.
    pub skipped: usize,
    /// Analyzed pairs whose assessment is `violation-candidate`.
    pub violation_candidates: usize,
}

/// Run one survey pass.
///
/// The run reads the cached index, selects candidates, drops packages that the
/// output file already covers, probes docs, and appends a record per remaining
/// package. It returns when every pending package has a record; a file or
/// network failure aborts the run with the records written so far intact for
/// the next resume.
pub fn run(options: &SurveyOptions) -> Result<SurveySummary, SurveyError> {
    let rows = read_index(&options.cache_dir)?;
    let candidates = select_candidates(&rows, options.shard, options.limit)?;
    let selected = candidates.len();
    let resume = read_resume(&options.output)?;
    let pending: Vec<PackageCandidate> = candidates
        .into_iter()
        .filter(|candidate| !resume.index.is_package_done(&candidate.name))
        .collect();

    // Open the output before the early return so a torn final line left by a
    // killed run is repaired even when every package is already recorded.
    let file = open_output(&resume, &options.output)?;
    let writer = JsonlWriter::new(file);
    if pending.is_empty() {
        return Ok(SurveySummary {
            packages: selected,
            analyzed: 0,
            skipped: 0,
            violation_candidates: 0,
        });
    }

    let client = HackageClient::new(options.cache_dir.clone(), options.offline, options.refresh)?;
    let cursor = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let progress = Progress::new(pending.len());
    eprintln!(
        "pvpdiff survey: {selected} packages selected, {} pending after resume",
        pending.len()
    );

    let context = WorkerContext {
        candidates: &pending,
        cursor: &cursor,
        writer: &writer,
        options,
        progress: &progress,
        stop: &stop,
    };
    let totals = if options.jobs <= 1 {
        worker(client, &context)?
    } else {
        let workers = options.jobs.min(pending.len());
        run_parallel(&client, &context, workers)?
    };

    eprintln!(
        "pvpdiff survey: {} analyzed, {} skipped, {} violation candidates",
        totals.analyzed, totals.skipped, totals.violations
    );
    Ok(SurveySummary {
        packages: selected,
        analyzed: totals.analyzed,
        skipped: totals.skipped,
        violation_candidates: totals.violations,
    })
}

/// Run the worker loop on several threads.
fn run_parallel(
    client: &HackageClient,
    context: &WorkerContext<'_>,
    workers: usize,
) -> Result<WorkerTotals, SurveyError> {
    let outcomes = thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let client = client.clone();
            handles.push(scope.spawn(move || worker(client, context)));
        }

        let mut outcomes = Vec::with_capacity(workers);
        for handle in handles {
            let outcome = match handle.join() {
                Ok(outcome) => outcome,
                Err(panic) => std::panic::resume_unwind(panic),
            };
            outcomes.push(outcome);
        }
        outcomes
    });

    let mut totals = WorkerTotals::default();
    for outcome in outcomes {
        totals.add(outcome?);
    }
    Ok(totals)
}

/// Append-only JSONL writer shared by the workers.
struct JsonlWriter {
    file: Mutex<File>,
}

impl JsonlWriter {
    fn new(file: File) -> Self {
        Self {
            file: Mutex::new(file),
        }
    }

    /// Append one record.
    fn append(&self, record: &SurveyRecord) -> Result<(), SurveyError> {
        // Recovering a poisoned lock keeps the other workers from panicking on
        // the same signal; the run reports the first error it saw.
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        append_record(&mut *file, record)?;
        file.flush()?;
        Ok(())
    }
}

/// State shared with every worker.
struct WorkerContext<'a> {
    candidates: &'a [PackageCandidate],
    cursor: &'a AtomicUsize,
    writer: &'a JsonlWriter,
    options: &'a SurveyOptions,
    progress: &'a Progress,
    stop: &'a AtomicBool,
}

/// Counts accumulated by one worker.
#[derive(Debug, Default)]
struct WorkerTotals {
    analyzed: usize,
    skipped: usize,
    violations: usize,
}

impl WorkerTotals {
    fn add(&mut self, other: Self) {
        self.analyzed += other.analyzed;
        self.skipped += other.skipped;
        self.violations += other.violations;
    }
}

/// Progress reporting on stderr.
struct Progress {
    total: usize,
    completed: AtomicUsize,
}

impl Progress {
    fn new(total: usize) -> Self {
        Self {
            total,
            completed: AtomicUsize::new(0),
        }
    }

    /// Count one finished package and report at the fixed interval.
    fn tick(&self) {
        let finished = self.completed.fetch_add(1, Ordering::Relaxed) + 1;
        if finished.is_multiple_of(PROGRESS_EVERY) {
            let total = self.total;
            eprintln!("pvpdiff survey: {finished}/{total} packages done");
        }
    }
}

/// Pacing between fetches inside one worker.
struct Pacing {
    delay: Duration,
    started: bool,
}

impl Pacing {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            started: false,
        }
    }

    /// Wait before the next fetch, unless this is the first one.
    fn wait(&mut self) {
        if self.started && !self.delay.is_zero() {
            sleep(self.delay);
        }
        self.started = true;
    }
}

/// Loop over the candidate list, claiming one package at a time.
fn worker(client: HackageClient, context: &WorkerContext<'_>) -> Result<WorkerTotals, SurveyError> {
    let mut totals = WorkerTotals::default();
    let mut pacing = Pacing::new(context.options.delay);
    loop {
        if context.stop.load(Ordering::Relaxed) {
            break;
        }
        let index = context.cursor.fetch_add(1, Ordering::Relaxed);
        let Some(candidate) = context.candidates.get(index) else {
            break;
        };

        let (record, violation) = process_package(
            &client,
            candidate,
            context.options.max_versions,
            &mut pacing,
        );
        if let Err(error) = context.writer.append(&record) {
            context.stop.store(true, Ordering::Relaxed);
            return Err(error);
        }
        match record {
            SurveyRecord::Analyzed(_) => {
                totals.analyzed += 1;
                if violation {
                    totals.violations += 1;
                }
            }
            SurveyRecord::Skipped(_) => totals.skipped += 1,
        }
        context.progress.tick();
    }
    Ok(totals)
}

/// Fetch the newest two Hoogle databases for a package.
fn probe_pair(
    client: &HackageClient,
    package: &PackageCandidate,
    max_versions: usize,
    pacing: &mut Pacing,
) -> ProbeOutcome {
    let mut probed = Vec::new();
    let mut docs = Vec::new();
    for version in package.versions.iter().take(max_versions) {
        probed.push(version.text.clone());
        pacing.wait();
        match client.fetch_docs(&package.name, &version.text) {
            Ok(artifact) => docs.push(DocsSide {
                version: version.clone(),
                bytes: artifact.bytes,
                sha256: artifact.sha256,
            }),
            Err(error) if docs_absent(&error) => continue,
            Err(_) => {
                return ProbeOutcome::Failed {
                    reason: REASON_FETCH,
                    probed,
                };
            }
        }
        if docs.len() == DOCS_NEEDED {
            break;
        }
    }

    if docs.len() == DOCS_NEEDED {
        // `docs` is newest first, so popping yields the older side and then the
        // newer one.
        let mut docs = docs;
        let older = docs.pop().expect("probe collected two docs");
        let newer = docs.pop().expect("probe collected two docs");
        return ProbeOutcome::Pair {
            older,
            newer,
            probed,
        };
    }
    ProbeOutcome::FewDocs { probed }
}

/// Whether a fetch error proves the database is absent rather than failed.
fn docs_absent(error: &HackageError) -> bool {
    matches!(
        error,
        HackageError::DocsUnavailable { .. } | HackageError::Offline(_)
    )
}

/// One Hoogle database in memory.
struct DocsSide {
    version: CandidateVersion,
    bytes: Vec<u8>,
    sha256: String,
}

/// Result of probing one package.
enum ProbeOutcome {
    /// The two newest databases that exist.
    Pair {
        older: DocsSide,
        newer: DocsSide,
        probed: Vec<String>,
    },
    /// Fewer than two databases exist in the probed window.
    FewDocs { probed: Vec<String> },
    /// A fetch failed in a way that does not prove absence.
    Failed {
        reason: &'static str,
        probed: Vec<String>,
    },
}

/// Build one record for a package.
fn process_package(
    client: &HackageClient,
    package: &PackageCandidate,
    max_versions: usize,
    pacing: &mut Pacing,
) -> (SurveyRecord, bool) {
    match probe_pair(client, package, max_versions, pacing) {
        ProbeOutcome::Pair {
            older,
            newer,
            probed,
        } => analyze(package, older, newer, probed),
        ProbeOutcome::FewDocs { probed } => (skipped(package, REASON_DOCS, probed), false),
        ProbeOutcome::Failed { reason, probed } => (skipped(package, reason, probed), false),
    }
}

/// Diff the two databases and build the analyzed record.
fn analyze(
    package: &PackageCandidate,
    older: DocsSide,
    newer: DocsSide,
    probed: Vec<String>,
) -> (SurveyRecord, bool) {
    let (Ok(older_surface), Ok(newer_surface)) = (
        parse_hoogle_bytes(&older.bytes),
        parse_hoogle_bytes(&newer.bytes),
    ) else {
        return (skipped(package, REASON_PARSE, probed), false);
    };

    let older_api = PackageApi {
        surface: older_surface,
        cabal: None,
    };
    let newer_api = PackageApi {
        surface: newer_surface,
        cabal: None,
    };
    let result = diff(&older_api, &newer_api, DiffOptions::default());
    let classification = result.classification();
    let counts = result.counts();
    let observed = observed_bump(&older.version.parsed, &newer.version.parsed);
    let assessment = assess(classification, observed);
    let record = AnalyzedRecord {
        schema: RECORD_SCHEMA,
        package: package.name.clone(),
        from: older.version.text.clone(),
        to: newer.version.text.clone(),
        classification,
        observed_bump: observed,
        assessment,
        counts,
        rules: rule_histogram(&result),
        change_count: result.changes.len(),
        unknown_count: counts.unknown,
        docs_sha256: DocsDigests {
            older: older.sha256,
            newer: newer.sha256,
        },
        diagnostics: diagnostic_codes(&result),
    };
    let violation = assessment == Assessment::ViolationCandidate;
    (SurveyRecord::Analyzed(record), violation)
}

/// Build the counted rule histogram of one diff.
fn rule_histogram(result: &DiffResult) -> BTreeMap<String, usize> {
    let mut rules = BTreeMap::new();
    for change in &result.changes {
        if change.counted {
            *rules.entry(change.rule.clone()).or_insert(0) += 1;
        }
    }
    rules
}

/// Diagnostic codes of one diff, sorted and deduplicated.
fn diagnostic_codes(result: &DiffResult) -> Vec<String> {
    let mut codes: Vec<String> = result
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

/// Build a skipped record.
fn skipped(package: &PackageCandidate, reason: &str, probed: Vec<String>) -> SurveyRecord {
    SurveyRecord::Skipped(SkippedRecord {
        schema: RECORD_SCHEMA,
        package: package.name.clone(),
        reason: reason.to_string(),
        probed,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use pvpdiff_core::{Assessment, Bump, Classification};
    use pvpdiff_index::IndexRow;

    use super::*;

    /// Hoogle database header shared by every fixture body.
    const HEADER: &str = "-- Hoogle documentation, generated by Haddock\n-- See Hoogle, http://www.haskell.org/hoogle/\n\n";

    /// Build a minimal valid Hoogle database.
    fn database(package: &str, version: &str, body: &str) -> String {
        format!("{HEADER}@package {package}\n@version {version}\n\nmodule Demo\n\n{body}\n")
    }

    /// Write the index JSONL the cache reader expects.
    fn write_index(cache: &Path, rows: &[(&str, &str)]) {
        let directory = cache.join("index");
        fs::create_dir_all(&directory).expect("create index dir");
        let mut text = String::new();
        for (package, version) in rows {
            let row = IndexRow {
                package: (*package).to_string(),
                version: (*version).to_string(),
                conditional: false,
                dependencies: Vec::new(),
                cabal_sha256: String::new(),
            };
            text.push_str(&serde_json::to_string(&row).expect("index row"));
            text.push('\n');
        }
        fs::write(directory.join("index-v1.jsonl"), text).expect("write index");
    }

    /// Seed one cached Hoogle database and return its digest.
    fn seed_docs(cache: &Path, package: &str, version: &str, body: &str) -> String {
        let directory = cache.join("hackage").join(package).join(version);
        fs::create_dir_all(&directory).expect("create docs dir");
        let name = format!("{package}.txt");
        let text = database(package, version, body);
        fs::write(directory.join(&name), &text).expect("write docs");
        let sha256 = pvpdiff_hackage::sha256_hex(text.as_bytes());
        let meta = serde_json::json!({
            "url": format!("https://example.test/{package}/{version}"),
            "etag": null,
            "last_modified": null,
            "sha256": sha256,
            "fetched_at": 0,
        });
        fs::write(
            directory.join(format!("{name}.meta.json")),
            serde_json::to_vec(&meta).expect("meta JSON"),
        )
        .expect("write meta");
        sha256
    }

    /// Build the four-package fixture used by the run tests.
    ///
    /// `demo` has docs at 4.0.0 and 2.0.0, `minorbreak` has a breaking change
    /// with a minor bump, `deep` has docs only below the probe window, and
    /// `onedoc` has a single version.
    fn fixture(workspace: &Path) -> (SurveyOptions, String, String) {
        let cache = workspace.join("cache");
        write_index(
            &cache,
            &[
                ("deep", "4.0.0"),
                ("deep", "3.0.0"),
                ("deep", "2.0.0"),
                ("deep", "1.0.0"),
                ("demo", "4.0.0"),
                ("demo", "3.0.0"),
                ("demo", "2.0.0"),
                ("demo", "1.0.0"),
                ("minorbreak", "1.0.1"),
                ("minorbreak", "1.0.0"),
                ("onedoc", "1.0.0"),
            ],
        );
        let newer_sha = seed_docs(&cache, "demo", "4.0.0", "foo :: Int -> Int");
        let older_sha = seed_docs(&cache, "demo", "2.0.0", "foo :: Int");
        seed_docs(&cache, "demo", "1.0.0", "foo :: Int -> Int");
        seed_docs(&cache, "deep", "2.0.0", "foo :: Int");
        seed_docs(&cache, "deep", "1.0.0", "foo :: Int -> Int");
        seed_docs(&cache, "minorbreak", "1.0.1", "foo :: String");
        seed_docs(&cache, "minorbreak", "1.0.0", "foo :: Int");
        seed_docs(&cache, "onedoc", "1.0.0", "foo :: Int");

        let options = SurveyOptions {
            cache_dir: cache,
            output: workspace.join("survey-v1.jsonl"),
            limit: None,
            jobs: 0,
            delay: Duration::ZERO,
            offline: true,
            refresh: false,
            shard: None,
            max_versions: 3,
        };
        (options, older_sha, newer_sha)
    }

    /// Read every record of a JSONL file.
    fn output_records(path: &Path) -> Vec<SurveyRecord> {
        let text = fs::read_to_string(path).expect("read output");
        text.lines()
            .map(|line| serde_json::from_str(line).expect("survey record"))
            .collect()
    }

    /// Pick the record for one package.
    fn record_for<'a>(records: &'a [SurveyRecord], package: &str) -> &'a SurveyRecord {
        records
            .iter()
            .find(|record| record.package() == package)
            .expect("record for package")
    }

    /// Check that an offline run selects the newest two databases, classifies
    /// the pair, counts the rule, and records skips for the packages without a
    /// second database.
    #[test]
    fn runs_offline_and_writes_records() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let (options, older_sha, newer_sha) = fixture(workspace.path());
        let summary = run(&options).expect("survey run");
        assert_eq!(summary.packages, 4);
        assert_eq!(summary.analyzed, 2);
        assert_eq!(summary.skipped, 2);
        assert_eq!(summary.violation_candidates, 1);

        let records = output_records(&options.output);
        assert_eq!(records.len(), 4);

        match record_for(&records, "demo") {
            SurveyRecord::Analyzed(record) => {
                assert_eq!(record.from, "2.0.0");
                assert_eq!(record.to, "4.0.0");
                assert_eq!(record.classification, Classification::Breaking);
                assert_eq!(record.observed_bump, Bump::Major);
                assert_eq!(record.assessment, Assessment::Consistent);
                assert_eq!(record.rules.get("pvp/signature-changed"), Some(&1));
                assert_eq!(record.change_count, 1);
                assert_eq!(record.unknown_count, 0);
                assert_eq!(record.counts.breaking, 1);
                assert_eq!(record.docs_sha256.older, older_sha);
                assert_eq!(record.docs_sha256.newer, newer_sha);
                assert!(record.diagnostics.is_empty());
            }
            other => panic!("demo should be analyzed, got {other:?}"),
        }

        match record_for(&records, "minorbreak") {
            SurveyRecord::Analyzed(record) => {
                assert_eq!(record.from, "1.0.0");
                assert_eq!(record.to, "1.0.1");
                assert_eq!(record.observed_bump, Bump::Minor);
                assert_eq!(record.assessment, Assessment::ViolationCandidate);
            }
            other => panic!("minorbreak should be analyzed, got {other:?}"),
        }

        match record_for(&records, "deep") {
            SurveyRecord::Skipped(record) => {
                assert_eq!(record.reason, REASON_DOCS);
                assert_eq!(record.probed, vec!["4.0.0", "3.0.0", "2.0.0"]);
            }
            other => panic!("deep should be skipped, got {other:?}"),
        }
        match record_for(&records, "onedoc") {
            SurveyRecord::Skipped(record) => {
                assert_eq!(record.reason, REASON_DOCS);
                assert_eq!(record.probed, vec!["1.0.0"]);
            }
            other => panic!("onedoc should be skipped, got {other:?}"),
        }
    }

    /// Check that the run limit takes the first packages in sorted order and
    /// that the summary counts only the records the run appended.
    #[test]
    fn applies_run_limit() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let (mut options, _, _) = fixture(workspace.path());
        options.limit = Some(2);
        let summary = run(&options).expect("limited run");
        assert_eq!(summary.packages, 2);
        assert_eq!(summary.analyzed, 1);
        assert_eq!(summary.skipped, 1);

        let records = output_records(&options.output);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].package(), "deep");
        assert_eq!(records[1].package(), "demo");
    }

    /// Check that a run with several workers appends the same records as a
    /// sequential run and that no line interleaves.
    #[test]
    fn parallel_run_matches_sequential_run() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let (sequential, _, _) = fixture(workspace.path());
        let sequential_summary = run(&sequential).expect("sequential run");

        let mut parallel = sequential.clone();
        parallel.jobs = 4;
        parallel.output = workspace.path().join("parallel-v1.jsonl");
        let parallel_summary = run(&parallel).expect("parallel run");
        assert_eq!(parallel_summary, sequential_summary);

        let sequential_lines: BTreeSet<String> = output_records(&sequential.output)
            .iter()
            .map(|record| serde_json::to_string(record).expect("record JSON"))
            .collect();
        let parallel_lines: BTreeSet<String> = output_records(&parallel.output)
            .iter()
            .map(|record| serde_json::to_string(record).expect("record JSON"))
            .collect();
        assert_eq!(parallel_lines, sequential_lines);
        assert_eq!(parallel_lines.len(), 4);
    }

    /// Check that a second run skips completed packages and leaves the file
    /// byte for byte unchanged, and that a torn final line is truncated.
    #[test]
    fn resumes_completed_packages() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let (options, _, _) = fixture(workspace.path());
        let first = run(&options).expect("first run");
        assert_eq!(first.analyzed, 2);
        let original = fs::read(&options.output).expect("read first output");

        let second = run(&options).expect("second run");
        assert_eq!(second.packages, 4);
        assert_eq!(second.analyzed, 0);
        assert_eq!(second.skipped, 0);
        assert_eq!(
            fs::read(&options.output).expect("read second output"),
            original
        );

        let mut torn = original.clone();
        torn.extend_from_slice(b"{\"schema\":1,\"outcome\":\"ana");
        fs::write(&options.output, &torn).expect("write torn output");
        let third = run(&options).expect("third run");
        assert_eq!(third.analyzed, 0);
        assert_eq!(
            fs::read(&options.output).expect("read repaired output"),
            original
        );
        assert_eq!(output_records(&options.output).len(), 4);
    }
}
