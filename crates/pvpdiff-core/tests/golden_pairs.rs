//! Golden-pair tests over the pinned Hackage fixtures.
//!
//! The fixtures are fetched by `scripts/fetch-fixtures.sh`. When
//! `fixtures/real/` is absent the test prints a skip notice and passes, so a
//! clean checkout and the Nix sandbox work offline.

use std::path::{Path, PathBuf};

use pvpdiff_core::diff::{DiffOptions, PackageApi};
use pvpdiff_core::report::SideArtifacts;
use pvpdiff_core::{Assessment, Bump, Classification, build_report, diff};
use pvpdiff_parse::{parse_cabal, parse_hoogle_bytes};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expected {
    package: String,
    from: String,
    to: String,
    classification: Classification,
    observed_bump: Bump,
    assessment: Assessment,
    suggested_version: Option<String>,
    #[serde(default)]
    counts_at_least: MinimumCounts,
    #[serde(default)]
    required_rules: Vec<String>,
    #[serde(default)]
    forbidden_rules: Vec<String>,
    #[serde(default)]
    required_changes: Vec<RequiredChange>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MinimumCounts {
    #[serde(default)]
    breaking: usize,
    #[serde(default)]
    additive: usize,
    #[serde(default)]
    unknown: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredChange {
    rule: String,
    name: String,
    module: String,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn load_pair(expected: &Expected) -> Option<(PackageApi, PackageApi)> {
    let root = repo_root();
    let old_dir = root
        .join("fixtures/real")
        .join(&expected.package)
        .join(&expected.from);
    let new_dir = root
        .join("fixtures/real")
        .join(&expected.package)
        .join(&expected.to);
    let old_docs = old_dir.join(format!("{}.txt", expected.package));
    let new_docs = new_dir.join(format!("{}.txt", expected.package));
    if !old_docs.is_file() || !new_docs.is_file() {
        eprintln!(
            "skip golden pair {} {} -> {}: fixtures/real is absent (run scripts/fetch-fixtures.sh)",
            expected.package, expected.from, expected.to
        );
        return None;
    }
    let old_surface = parse_hoogle_bytes(&std::fs::read(&old_docs).expect("read old docs"))
        .expect("parse old docs");
    let new_surface = parse_hoogle_bytes(&std::fs::read(&new_docs).expect("read new docs"))
        .expect("parse new docs");

    let old_cabal = read_cabal(&old_dir, &expected.package);
    let new_cabal = read_cabal(&new_dir, &expected.package);
    Some((
        PackageApi {
            surface: old_surface,
            cabal: old_cabal,
        },
        PackageApi {
            surface: new_surface,
            cabal: new_cabal,
        },
    ))
}

fn read_cabal(directory: &Path, package: &str) -> Option<pvpdiff_parse::CabalMeta> {
    let path = directory.join(format!("{package}.cabal"));
    let text = std::fs::read_to_string(path).ok()?;
    parse_cabal(&text).meta
}

fn expected_files() -> Vec<PathBuf> {
    let directory = repo_root().join("fixtures/expected");
    let mut files: Vec<PathBuf> = std::fs::read_dir(directory)
        .expect("expected fixture directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files
}

/// Check every curated pair against the classifier. The test asserts the
/// aggregate verdict, PVP assessment, suggested version, required and
/// forbidden rules, required named changes, and minimum counts.
#[test]
fn golden_pairs_match_curated_expectations() {
    let mut checked = 0usize;
    for path in expected_files() {
        let expected: Expected =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read expected fixture"))
                .expect("parse expected fixture");
        let Some((old, new)) = load_pair(&expected) else {
            continue;
        };
        checked += 1;
        let result = diff(&old, &new, DiffOptions::default());
        let report = build_report(
            &old,
            SideArtifacts::default(),
            &new,
            SideArtifacts::default(),
            &result,
            Vec::new(),
        );
        assert_eq!(
            report.classification,
            expected.classification,
            "{}: classification",
            path.display()
        );
        assert_eq!(
            report.pvp.observed_bump,
            Some(expected.observed_bump),
            "{}: observed bump",
            path.display()
        );
        assert_eq!(
            report.pvp.assessment,
            expected.assessment,
            "{}: assessment",
            path.display()
        );
        assert_eq!(
            report.pvp.suggested_version,
            expected.suggested_version,
            "{}: suggested version",
            path.display()
        );
        assert_eq!(
            report.package,
            expected.package,
            "{}: package",
            path.display()
        );

        let counted: Vec<_> = report
            .changes
            .iter()
            .filter(|change| change.counted)
            .collect();
        if expected.classification != Classification::Breaking {
            assert_eq!(
                report.counts.breaking,
                0,
                "{}: no breaking changes expected",
                path.display()
            );
        }
        let counts = report.counts;
        assert!(
            counts.breaking >= expected.counts_at_least.breaking,
            "{}: breaking count {} < {}",
            path.display(),
            counts.breaking,
            expected.counts_at_least.breaking
        );
        assert!(
            counts.additive >= expected.counts_at_least.additive,
            "{}: additive count {} < {}",
            path.display(),
            counts.additive,
            expected.counts_at_least.additive
        );
        assert!(
            counts.unknown >= expected.counts_at_least.unknown,
            "{}: unknown count {} < {}",
            path.display(),
            counts.unknown,
            expected.counts_at_least.unknown
        );

        let rules: Vec<&str> = counted.iter().map(|change| change.rule.as_str()).collect();
        for rule in &expected.required_rules {
            assert!(
                rules.contains(&rule.as_str()),
                "{}: missing required rule {rule}",
                path.display()
            );
        }
        for rule in &expected.forbidden_rules {
            assert!(
                !rules.contains(&rule.as_str()),
                "{}: forbidden rule {rule} present",
                path.display()
            );
        }
        for required in &expected.required_changes {
            let found = counted.iter().any(|change| {
                change.rule == required.rule
                    && change.name.as_deref() == Some(required.name.as_str())
                    && change
                        .modules
                        .iter()
                        .any(|module| module == &required.module)
            });
            assert!(
                found,
                "{}: missing required change {} {} in {}",
                path.display(),
                required.rule,
                required.name,
                required.module
            );
        }
    }
    if checked == 0 {
        eprintln!("no golden pairs ran; fixtures/real is absent");
    }
}
