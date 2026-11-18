//! Reverse-dependency bounds evaluation.
//!
//! The blast radius answers one question: if the target package releases a new
//! version, which dependents' Cabal ranges exclude it. Each dependent is
//! evaluated at its newest version in the index database, or at the version a
//! Stackage snapshot pins when a snapshot is supplied. The status separates a
//! plain exclusion from the two cases worth acting on: a range with no upper
//! bound, and a minor-level bound that excludes a version a PVP-compatible
//! bound would admit.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use cabalist_parser::ast::{Version, VersionRange, parse_version_range};
use pvpdiff_core::HaskellVersion;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::index::{IndexError, IndexMeta, IndexRow};
use crate::stackage::SnapshotPins;

/// Schema version of the blast-radius report.
const BLAST_SCHEMA: u32 = 1;

const REASON_COMPATIBLE: &str = "range-admits-new-version";
const REASON_NO_UPPER_BOUND: &str = "no-upper-bound";
const REASON_UPPER_BOUND: &str = "upper-bound-below-new-version";
const REASON_LOWER_BOUND: &str = "lower-bound-above-new-version";
const REASON_MINOR_UPPER_BOUND: &str = "minor-level-upper-bound";
const REASON_EXACT_PIN: &str = "exact-pin";
const REASON_UNPARSEABLE: &str = "range-unparseable";
const REASON_EXCLUDES: &str = "range-excludes-new-version";

/// How one dependent's bounds relate to the proposed version.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BlastStatus {
    /// The range excludes the new version and no minor-level bound explains
    /// the exclusion.
    Excluded,
    /// The range admits the new version and carries an upper bound.
    Compatible,
    /// The range admits the new version but has no upper bound at all.
    NoUpperBound,
    /// The range excludes the new version through a minor-level bound that a
    /// PVP-compatible bound would admit.
    OverConstrained,
}

impl BlastStatus {
    /// Stable name used in JSON and counts.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Excluded => "excluded",
            Self::Compatible => "compatible",
            Self::NoUpperBound => "no-upper-bound",
            Self::OverConstrained => "over-constrained",
        }
    }
}

/// Dependent counts per status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlastCounts {
    /// Dependents whose range excludes the new version.
    pub excluded: usize,
    /// Dependents whose bounded range admits the new version.
    pub compatible: usize,
    /// Dependents whose unbounded range admits the new version.
    pub no_upper_bound: usize,
    /// Dependents whose minor-level bound excludes the new version.
    pub over_constrained: usize,
}

impl BlastCounts {
    /// Count the statuses in a dependent list.
    pub fn from_dependents(dependents: &[BlastDependent]) -> Self {
        let mut counts = Self::default();
        for dependent in dependents {
            match dependent.status {
                BlastStatus::Excluded => counts.excluded += 1,
                BlastStatus::Compatible => counts.compatible += 1,
                BlastStatus::NoUpperBound => counts.no_upper_bound += 1,
                BlastStatus::OverConstrained => counts.over_constrained += 1,
            }
        }
        counts
    }
}

/// Index provenance for a blast-radius report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlastSource {
    /// SHA-256 of the index archive, filled from the sync metadata.
    pub index_sha256: Option<String>,
    /// ETag of the index archive, filled from the sync metadata.
    pub index_etag: Option<String>,
    /// Number of packages in the index database.
    pub package_count: usize,
    /// Number of package versions in the index database.
    pub version_count: usize,
}

/// One dependency bound that a dependent declared on the target package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlastDependent {
    /// Dependent package name.
    pub package: String,
    /// Dependent package version that was evaluated.
    pub version: String,
    /// Declared version range on the target package.
    pub range: String,
    /// Component that declared the dependency.
    pub component: String,
    /// Status of this bound against the new version.
    pub status: BlastStatus,
    /// Stable explanation of the status.
    pub reason: String,
}

/// Reverse-dependency report for one proposed version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlastReport {
    /// Schema version.
    pub schema: u32,
    /// Target package name.
    pub package: String,
    /// Proposed version.
    pub new_version: String,
    /// Index provenance.
    pub source: BlastSource,
    /// Counts per status.
    pub counts: BlastCounts,
    /// Dependent bounds, sorted by package and version.
    pub dependents: Vec<BlastDependent>,
}

impl BlastReport {
    /// Attach the index digest and ETag from a completed sync.
    ///
    /// `blast_radius` only sees index rows, so it cannot know the archive
    /// provenance; the CLI fills it from `read_meta` before rendering.
    pub fn attach_index(&mut self, meta: &IndexMeta) {
        self.source.index_sha256 = Some(meta.index_sha256.clone());
        self.source.index_etag = meta.etag.clone();
        self.source.package_count = meta.packages;
        self.source.version_count = meta.versions;
    }

    /// Serialize the report as pretty JSON with a trailing newline.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }

    /// JSON Schema for the blast-radius report contract.
    pub fn schema_value() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(BlastReport)).unwrap_or(serde_json::Value::Null)
    }
}

/// Evaluate reverse-dependency bounds for one proposed version.
///
/// Every dependent package is evaluated at its newest indexed version, or at
/// the version pinned by `snapshot` when one is supplied. A package that is
/// absent from the snapshot is left out of the report.
pub fn blast_radius(
    rows: &[IndexRow],
    target: &str,
    new_version: &str,
    snapshot: Option<&SnapshotPins>,
) -> Result<BlastReport, IndexError> {
    let proposed = Version::parse(new_version)
        .ok_or_else(|| IndexError::InvalidVersion(new_version.into()))?;
    let package_count = rows
        .iter()
        .map(|row| row.package.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let version_count = rows.len();

    let selected = latest_dependents(rows, target, snapshot);
    let mut dependents = Vec::new();
    for row in selected {
        for dependency in row
            .dependencies
            .iter()
            .filter(|dependency| packages_equal(&dependency.package, target))
        {
            let (status, reason) = classify(&dependency.range, &proposed);
            dependents.push(BlastDependent {
                package: row.package.clone(),
                version: row.version.clone(),
                range: dependency.range.clone(),
                component: dependency.component.clone(),
                status,
                reason: reason.to_string(),
            });
        }
    }
    dependents.sort();
    dependents.dedup();

    Ok(BlastReport {
        schema: BLAST_SCHEMA,
        package: target.to_string(),
        new_version: new_version.to_string(),
        source: BlastSource {
            index_sha256: None,
            index_etag: None,
            package_count,
            version_count,
        },
        counts: BlastCounts::from_dependents(&dependents),
        dependents,
    })
}

/// Select the newest version of every dependent package that depends on the
/// target.
fn latest_dependents<'a>(
    rows: &'a [IndexRow],
    target: &str,
    snapshot: Option<&SnapshotPins>,
) -> Vec<&'a IndexRow> {
    let pins = snapshot.map(pinned_versions);
    let mut latest: BTreeMap<String, (&'a IndexRow, HaskellVersion)> = BTreeMap::new();

    // Select the newest version of every package first. A package whose
    // newest version dropped the dependency is not a reverse dependency, even
    // if an older version still declared it.
    for row in rows {
        if packages_equal(&row.package, target) {
            continue;
        }
        let key = row.package.to_ascii_lowercase();
        let Ok(version) = HaskellVersion::parse(&row.version) else {
            continue;
        };

        // A snapshot narrows the candidate row to the pinned version, because
        // that is the version the snapshot would build. Cabal compares
        // versions numerically, so 2.1.4 and 2.1.4.0 are the same pin.
        if let Some(pins) = &pins {
            let Some(pinned) = pins.get(&key) else {
                continue;
            };
            let Ok(pinned) = HaskellVersion::parse(pinned) else {
                continue;
            };
            if pinned != version {
                continue;
            }
        }
        let replace = match latest.get(&key) {
            Some((_, newest)) => *newest < version,
            None => true,
        };
        if replace {
            latest.insert(key, (row, version));
        }
    }

    latest
        .into_values()
        .map(|(row, _)| row)
        .filter(|row| depends_on(row, target))
        .collect()
}

fn pinned_versions(snapshot: &SnapshotPins) -> BTreeMap<String, String> {
    snapshot
        .packages
        .iter()
        .map(|(name, version)| (name.to_ascii_lowercase(), version.clone()))
        .collect()
}

fn packages_equal(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn depends_on(row: &IndexRow, target: &str) -> bool {
    row.dependencies
        .iter()
        .any(|dependency| packages_equal(&dependency.package, target))
}

/// Classify one declared range against the proposed version.
fn classify(range_text: &str, proposed: &Version) -> (BlastStatus, &'static str) {
    let Some(range) = parse_version_range(range_text) else {
        // A compiled range that no longer parses must not look compatible.
        return (BlastStatus::Excluded, REASON_UNPARSEABLE);
    };
    if range.satisfies(proposed) {
        return if has_upper_bound(&range) {
            (BlastStatus::Compatible, REASON_COMPATIBLE)
        } else {
            (BlastStatus::NoUpperBound, REASON_NO_UPPER_BOUND)
        };
    }
    match exclusion(&range, proposed) {
        Some(Exclusion::MinorUpperBound) => {
            (BlastStatus::OverConstrained, REASON_MINOR_UPPER_BOUND)
        }
        Some(Exclusion::MinorExactPin) => (BlastStatus::OverConstrained, REASON_EXACT_PIN),
        Some(Exclusion::ExactPin) => (BlastStatus::Excluded, REASON_EXACT_PIN),
        Some(Exclusion::LowerBound) => (BlastStatus::Excluded, REASON_LOWER_BOUND),
        Some(Exclusion::UpperBound) => (BlastStatus::Excluded, REASON_UPPER_BOUND),
        Some(Exclusion::Empty) | None => (BlastStatus::Excluded, REASON_EXCLUDES),
    }
}

/// Whether the range has an upper bound in any admitted branch.
fn has_upper_bound(range: &VersionRange) -> bool {
    match range {
        VersionRange::Any | VersionRange::Gt(_) | VersionRange::Gte(_) => false,
        VersionRange::NoVersion
        | VersionRange::Eq(_)
        | VersionRange::Lt(_)
        | VersionRange::Lte(_)
        | VersionRange::MajorBound(_) => true,
        VersionRange::And(left, right) => has_upper_bound(left) || has_upper_bound(right),
        VersionRange::Or(left, right) => has_upper_bound(left) && has_upper_bound(right),
    }
}

/// Why one bound excludes the proposed version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exclusion {
    /// A lower bound is above the proposed version.
    LowerBound,
    /// An upper bound is below the proposed version.
    UpperBound,
    /// An upper bound at minor granularity is below the proposed version.
    MinorUpperBound,
    /// An exact pin does not match the proposed version.
    ExactPin,
    /// A minor-level exact pin does not match the proposed version.
    MinorExactPin,
    /// The range admits no version at all.
    Empty,
}

/// Find the bound that excludes the proposed version, when the range is a
/// conjunction both sides must hold; for a disjunction every branch must
/// exclude, so the leftmost cause is reported.
fn exclusion(range: &VersionRange, proposed: &Version) -> Option<Exclusion> {
    match range {
        VersionRange::Any => None,
        VersionRange::NoVersion => Some(Exclusion::Empty),
        VersionRange::Eq(bound) => {
            if compare_versions(bound, proposed) == Ordering::Equal {
                None
            } else if minor_level_bound(bound, proposed) {
                Some(Exclusion::MinorExactPin)
            } else {
                Some(Exclusion::ExactPin)
            }
        }
        VersionRange::Gt(bound) => {
            if compare_versions(proposed, bound) == Ordering::Greater {
                None
            } else {
                Some(Exclusion::LowerBound)
            }
        }
        VersionRange::Gte(bound) => {
            if compare_versions(proposed, bound) != Ordering::Less {
                None
            } else {
                Some(Exclusion::LowerBound)
            }
        }
        VersionRange::Lt(bound) => {
            if compare_versions(proposed, bound) == Ordering::Less {
                None
            } else if minor_level_bound(bound, proposed) {
                Some(Exclusion::MinorUpperBound)
            } else {
                Some(Exclusion::UpperBound)
            }
        }
        VersionRange::Lte(bound) => {
            if compare_versions(proposed, bound) != Ordering::Greater {
                None
            } else if minor_level_bound(bound, proposed) {
                Some(Exclusion::MinorUpperBound)
            } else {
                Some(Exclusion::UpperBound)
            }
        }
        VersionRange::MajorBound(bound) => {
            let upper = major_upper(bound);
            if compare_versions(proposed, bound) == Ordering::Less {
                Some(Exclusion::LowerBound)
            } else if compare_versions(proposed, &upper) != Ordering::Less {
                Some(Exclusion::UpperBound)
            } else {
                None
            }
        }
        VersionRange::And(left, right) => {
            exclusion(left, proposed).or_else(|| exclusion(right, proposed))
        }
        VersionRange::Or(left, right) => {
            match (exclusion(left, proposed), exclusion(right, proposed)) {
                (None, _) | (_, None) => None,
                (Some(cause), _) => Some(cause),
            }
        }
    }
}

/// Cabal compares versions component by component and treats missing trailing
/// components as zero, so `2.1` and `2.1.0` are equal.
fn compare_versions(left: &Version, right: &Version) -> Ordering {
    let length = left.components.len().max(right.components.len());
    for index in 0..length {
        let left_component = left.components.get(index).copied().unwrap_or(0);
        let right_component = right.components.get(index).copied().unwrap_or(0);
        match left_component.cmp(&right_component) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

fn major_pair(version: &Version) -> (u64, u64) {
    (
        version.components.first().copied().unwrap_or(0),
        version.components.get(1).copied().unwrap_or(0),
    )
}

/// A bound is minor-level when it names at least three components inside the
/// proposed version's major pair.
fn minor_level_bound(bound: &Version, proposed: &Version) -> bool {
    bound.components.len() >= 3 && major_pair(bound) == major_pair(proposed)
}

/// The exclusive upper end of `^>=bound`, matching Cabal's rule.
fn major_upper(bound: &Version) -> Version {
    let mut components = bound.components.clone();
    if components.len() >= 2 {
        components[1] += 1;
        components.truncate(2);
    } else if let Some(first) = components.first_mut() {
        *first += 1;
    }
    Version { components }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexDependency;

    fn row(package: &str, version: &str, ranges: &[&str]) -> IndexRow {
        IndexRow {
            package: package.to_string(),
            version: version.to_string(),
            conditional: false,
            dependencies: ranges
                .iter()
                .map(|range| IndexDependency {
                    package: "text".to_string(),
                    range: (*range).to_string(),
                    component: "library".to_string(),
                })
                .collect(),
            cabal_sha256: format!("{package}-{version}"),
        }
    }

    fn status(range: &str, new_version: &str) -> (BlastStatus, &'static str) {
        classify(range, &Version::parse(new_version).expect("version"))
    }

    /// Check that the newest indexed version of a dependent is the one
    /// evaluated, that an unparsable version is skipped, and that packages
    /// without a target dependency stay out of the report.
    #[test]
    fn evaluates_latest_dependent_version() {
        let rows = vec![
            row("alpha", "0.9.0", &["^>=2.0"]),
            row("alpha", "2.0.0", &["^>=2.1"]),
            row("nodep", "1.0.0", &[]),
            row("bogus", "not-a-version", &["^>=2.0"]),
            row("text", "9.9.9", &["^>=2.0"]),
        ];
        let report = blast_radius(&rows, "text", "2.1.4", None).expect("report");
        let alpha: Vec<&BlastDependent> = report
            .dependents
            .iter()
            .filter(|dependent| dependent.package == "alpha")
            .collect();
        assert_eq!(alpha.len(), 1);
        assert_eq!(alpha[0].version, "2.0.0");
        assert_eq!(alpha[0].status, BlastStatus::Compatible);
        assert!(
            report
                .dependents
                .iter()
                .all(|dependent| dependent.package != "bogus"
                    && dependent.package != "nodep"
                    && dependent.package != "text")
        );
        assert_eq!(report.counts.compatible, 1);
    }

    /// Check the status matrix for excluded, compatible, no-upper-bound, and
    /// over-constrained bounds, including a package with two library bounds.
    #[test]
    fn classifies_dependents() {
        let rows = vec![
            row("beta", "1.0.0", &["^>=2.0"]),
            row("gamma", "1.0.0", &[">=2.0"]),
            row("delta", "1.0.0", &["<2.1.4"]),
            row("epsilon", "1.0.0", &[">=2.1.4 && <3"]),
            row("zeta", "1.0.0", &["==2.1.3"]),
            row("eta", "1.0.0", &[">=2.5"]),
            row("theta", "1.0.0", &["-any"]),
            row("iota", "1.0.0", &["^>=2.0 && <2.1"]),
            row("kappa", "1.0.0", &["<2.1"]),
            row("mu", "1.0.0", &["^>=2.0", ">=2.0"]),
        ];
        let report = blast_radius(&rows, "text", "2.1.4", None).expect("report");

        let by_package = |package: &str| -> Vec<&BlastDependent> {
            report
                .dependents
                .iter()
                .filter(|dependent| dependent.package == package)
                .collect()
        };
        assert_eq!(by_package("beta")[0].status, BlastStatus::Excluded);
        assert_eq!(
            by_package("beta")[0].reason,
            "upper-bound-below-new-version"
        );
        assert_eq!(by_package("gamma")[0].status, BlastStatus::NoUpperBound);
        assert_eq!(by_package("gamma")[0].reason, "no-upper-bound");
        assert_eq!(by_package("delta")[0].status, BlastStatus::OverConstrained);
        assert_eq!(by_package("delta")[0].reason, "minor-level-upper-bound");
        assert_eq!(by_package("epsilon")[0].status, BlastStatus::Compatible);
        assert_eq!(by_package("epsilon")[0].reason, "range-admits-new-version");
        assert_eq!(by_package("zeta")[0].status, BlastStatus::OverConstrained);
        assert_eq!(by_package("zeta")[0].reason, "exact-pin");
        assert_eq!(by_package("eta")[0].status, BlastStatus::Excluded);
        assert_eq!(by_package("eta")[0].reason, "lower-bound-above-new-version");
        assert_eq!(by_package("theta")[0].status, BlastStatus::NoUpperBound);
        assert_eq!(by_package("iota")[0].status, BlastStatus::Excluded);
        assert_eq!(
            by_package("iota")[0].reason,
            "upper-bound-below-new-version"
        );
        assert_eq!(by_package("kappa")[0].status, BlastStatus::Excluded);
        assert_eq!(
            by_package("kappa")[0].reason,
            "upper-bound-below-new-version"
        );
        assert_eq!(by_package("mu").len(), 2);

        assert_eq!(report.counts.excluded, 5);
        assert_eq!(report.counts.compatible, 1);
        assert_eq!(report.counts.no_upper_bound, 3);
        assert_eq!(report.counts.over_constrained, 2);
        assert_eq!(
            report.counts.excluded
                + report.counts.compatible
                + report.counts.no_upper_bound
                + report.counts.over_constrained,
            report.dependents.len()
        );

        // Dependents sort by package and version, then by the remaining fields.
        let mut sorted = report.dependents.clone();
        sorted.sort();
        assert_eq!(sorted, report.dependents);
    }

    /// Check that a snapshot keeps only pinned packages and evaluates the
    /// pinned version instead of the newest indexed one.
    #[test]
    fn filters_by_snapshot_pins() {
        let rows = vec![
            row("alpha", "0.9.0", &["^>=2.0"]),
            row("alpha", "2.0.0", &["^>=2.1"]),
            row("beta", "2.0.0", &["^>=2.0"]),
            row("gamma", "1.0.0", &["-any"]),
        ];
        let snapshot = SnapshotPins {
            name: "lts-24.67".to_string(),
            packages: BTreeMap::from([
                ("alpha".to_string(), "0.9.0".to_string()),
                ("beta".to_string(), "2.0.0".to_string()),
            ]),
        };

        let pinned = blast_radius(&rows, "text", "2.1.4", Some(&snapshot)).expect("pinned report");
        let names: Vec<&str> = pinned
            .dependents
            .iter()
            .map(|dependent| dependent.package.as_str())
            .collect();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert_eq!(pinned.dependents[0].version, "0.9.0");
        assert_eq!(pinned.dependents[0].status, BlastStatus::Excluded);

        let newest = blast_radius(&rows, "text", "2.1.4", None).expect("newest report");
        assert!(
            newest
                .dependents
                .iter()
                .any(|dependent| dependent.package == "alpha" && dependent.version == "2.0.0")
        );
        assert!(
            newest
                .dependents
                .iter()
                .any(|dependent| dependent.package == "gamma")
        );
    }

    /// Check the range table the P3 contract names: `^>=`, wildcard pins,
    /// conjunctions, disjunctions, `-any`, and the minor-level boundary.
    #[test]
    fn classifies_version_ranges() {
        let cases: &[(&str, &str, BlastStatus, &str)] = &[
            (
                "^>=2.0",
                "2.0.5",
                BlastStatus::Compatible,
                REASON_COMPATIBLE,
            ),
            (
                "^>=2.1",
                "2.1.4",
                BlastStatus::Compatible,
                REASON_COMPATIBLE,
            ),
            ("^>=2.0", "2.1.4", BlastStatus::Excluded, REASON_UPPER_BOUND),
            (
                "==2.1.*",
                "2.1.4",
                BlastStatus::Compatible,
                REASON_COMPATIBLE,
            ),
            (
                "==2.0.*",
                "2.1.4",
                BlastStatus::Excluded,
                REASON_UPPER_BOUND,
            ),
            (
                ">=2.0 && <3",
                "2.1.4",
                BlastStatus::Compatible,
                REASON_COMPATIBLE,
            ),
            (
                ">=2.0 && <2.1",
                "2.1.4",
                BlastStatus::Excluded,
                REASON_UPPER_BOUND,
            ),
            (
                ">=2.0 || >=3",
                "2.1.4",
                BlastStatus::NoUpperBound,
                REASON_NO_UPPER_BOUND,
            ),
            (
                "-any",
                "2.1.4",
                BlastStatus::NoUpperBound,
                REASON_NO_UPPER_BOUND,
            ),
            (
                "==2.1.4",
                "2.1.4",
                BlastStatus::Compatible,
                REASON_COMPATIBLE,
            ),
            (
                "<2.1.4",
                "2.1.4",
                BlastStatus::OverConstrained,
                REASON_MINOR_UPPER_BOUND,
            ),
            (
                "<=2.1.3",
                "2.1.4",
                BlastStatus::OverConstrained,
                REASON_MINOR_UPPER_BOUND,
            ),
            (
                "==2.1.3",
                "2.1.4",
                BlastStatus::OverConstrained,
                REASON_EXACT_PIN,
            ),
            ("<2.1", "2.1.4", BlastStatus::Excluded, REASON_UPPER_BOUND),
            ("<3.0", "2.1.4", BlastStatus::Compatible, REASON_COMPATIBLE),
            (
                "<2.1.5",
                "2.1.4",
                BlastStatus::Compatible,
                REASON_COMPATIBLE,
            ),
            (">=2.5", "2.1.4", BlastStatus::Excluded, REASON_LOWER_BOUND),
            ("-none", "2.1.4", BlastStatus::Excluded, REASON_EXCLUDES),
            (
                ">=2.1",
                "2.1.4",
                BlastStatus::NoUpperBound,
                REASON_NO_UPPER_BOUND,
            ),
        ];
        for (range, proposed, status, reason) in cases {
            assert_eq!(
                self::status(range, proposed),
                (*status, *reason),
                "range {range} against {proposed}"
            );
        }
    }

    /// Check that the report round-trips through JSON, fills provenance from
    /// the sync metadata, and exposes a schema.
    #[test]
    fn serializes_blast_report() {
        let rows = vec![row("alpha", "1.0.0", &["^>=2.0"])];
        let mut report = blast_radius(&rows, "text", "2.1.4", None).expect("report");
        assert_eq!(report.schema, 1);
        assert_eq!(report.source.index_sha256, None);

        let text = report.to_json().expect("json");
        assert!(text.starts_with('{'));
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\"schema\": 1"));
        assert!(text.contains("\"noUpperBound\""));
        assert!(text.contains("\"status\": \"excluded\""));
        assert!(text.contains("\"indexSha256\": null"));

        let decoded: BlastReport = serde_json::from_str(&text).expect("decode");
        assert_eq!(decoded, report);

        let meta = IndexMeta {
            index_url: "https://hackage.haskell.org/01-index.tar.gz".to_string(),
            index_sha256: "00".repeat(32),
            etag: Some("\"etag\"".to_string()),
            packages: 10,
            versions: 20,
            parse_skips: 1,
        };
        report.attach_index(&meta);
        assert_eq!(report.source.index_sha256, Some("00".repeat(32)));
        assert_eq!(report.source.index_etag.as_deref(), Some("\"etag\""));
        assert_eq!(report.source.version_count, 20);

        let schema = BlastReport::schema_value();
        let schema_text = schema.to_string();
        assert!(schema.get("properties").is_some());
        assert!(schema_text.contains("\"newVersion\""));
        assert!(schema_text.contains("\"no-upper-bound\""));
        assert!(schema_text.contains("\"noUpperBound\""));
    }

    /// Check that an invalid proposed version is a typed error, and that
    /// package names match case-insensitively.
    #[test]
    fn rejects_invalid_version_and_matches_names() {
        let rows = vec![IndexRow {
            package: "MixedCase".to_string(),
            version: "1.0.0".to_string(),
            conditional: false,
            dependencies: vec![IndexDependency {
                package: "Text".to_string(),
                range: "^>=2.0".to_string(),
                component: "library".to_string(),
            }],
            cabal_sha256: String::new(),
        }];
        let report = blast_radius(&rows, "text", "2.1.4", None).expect("report");
        assert_eq!(report.dependents.len(), 1);
        assert_eq!(report.dependents[0].package, "MixedCase");

        assert!(matches!(
            blast_radius(&rows, "text", "not-a-version", None),
            Err(IndexError::InvalidVersion(_))
        ));
    }
}
