//! Deterministic Markdown aggregation of a survey JSONL file.
//!
//! The report is derived only from the records in the file, so the same input
//! always renders the same text. Fixed rows for every classification, bump, and
//! assessment keep the table shape stable when a category is empty; rule rows
//! and skip reasons sort by count and then by name.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use pvpdiff_core::{Assessment, Bump, Classification};

use crate::SurveyError;
use crate::ensure_parent;
use crate::record::SurveyRecord;
use crate::record::read_records;

/// Rows shown in the rule table.
const TOP_RULES: usize = 10;
/// Scale for a share rendered as a percentage.
const PERCENT: f64 = 100.0;

/// Aggregate one survey file into Markdown.
///
/// `input` is the JSONL file a run appended to. When `output` is supplied the
/// same text is written there. The text ends with a newline and contains no
/// timestamps or paths, so it can be committed and diffed.
pub fn report(input: &Path, output: Option<&Path>) -> Result<String, SurveyError> {
    let records = read_records(input)?;
    let text = Aggregate::new(&records).render();
    if let Some(path) = output {
        ensure_parent(path)?;
        fs::write(path, &text)?;
    }
    Ok(text)
}

/// Totals and histograms over every record of one file.
#[derive(Debug, Default)]
struct Aggregate {
    records: usize,
    analyzed: usize,
    skipped: usize,
    classifications: BTreeMap<&'static str, usize>,
    bumps: BTreeMap<&'static str, usize>,
    assessments: BTreeMap<&'static str, usize>,
    rules: BTreeMap<String, usize>,
    skip_reasons: BTreeMap<String, usize>,
}

impl Aggregate {
    /// Count every record once.
    fn new(records: &[SurveyRecord]) -> Self {
        let mut aggregate = Self::default();
        for record in records {
            aggregate.add(record);
        }
        aggregate
    }

    /// Add one record to the totals and histograms.
    fn add(&mut self, record: &SurveyRecord) {
        self.records += 1;
        match record {
            SurveyRecord::Analyzed(record) => {
                self.analyzed += 1;
                *self
                    .classifications
                    .entry(record.classification.as_str())
                    .or_insert(0) += 1;
                *self
                    .bumps
                    .entry(bump_name(record.observed_bump))
                    .or_insert(0) += 1;
                *self
                    .assessments
                    .entry(record.assessment.as_str())
                    .or_insert(0) += 1;
                for (rule, count) in &record.rules {
                    *self.rules.entry(rule.clone()).or_insert(0) += count;
                }
            }
            SurveyRecord::Skipped(record) => {
                self.skipped += 1;
                *self.skip_reasons.entry(record.reason.clone()).or_insert(0) += 1;
            }
        }
    }

    /// Render the whole report.
    fn render(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "# PVP survey report");
        let _ = writeln!(text);
        let _ = writeln!(
            text,
            "Each analyzed record compares two package versions using their \
             Hoogle databases. Coverage counts the records a run attempted, so \
             the docs gap stays visible."
        );
        let _ = writeln!(text);

        let _ = writeln!(text, "## Coverage");
        let _ = writeln!(text);
        let _ = writeln!(text, "| Measure | Value |");
        let _ = writeln!(text, "|---|---:|");
        let _ = writeln!(text, "| Records | {} |", self.records);
        let _ = writeln!(text, "| Analyzed | {} |", self.analyzed);
        let _ = writeln!(text, "| Skipped | {} |", self.skipped);
        let _ = writeln!(
            text,
            "| Coverage | {} of records |",
            share(self.analyzed, self.records)
        );
        let violations = self.count_assessment(Assessment::ViolationCandidate);
        let _ = writeln!(
            text,
            "| Violation candidates | {} ({} of analyzed) |",
            violations,
            share(violations, self.analyzed)
        );
        let _ = writeln!(text);

        let classifications = [
            Classification::Breaking,
            Classification::Additive,
            Classification::None,
            Classification::Unknown,
        ];
        let _ = writeln!(text, "## Classifications");
        let _ = writeln!(text);
        let _ = writeln!(text, "| Classification | Records |");
        let _ = writeln!(text, "|---|---:|");
        for classification in classifications {
            let _ = writeln!(
                text,
                "| {} | {} |",
                classification.as_str(),
                self.classifications
                    .get(classification.as_str())
                    .copied()
                    .unwrap_or(0)
            );
        }
        let _ = writeln!(text);

        let bumps = [Bump::Major, Bump::Minor, Bump::Other, Bump::None];
        let _ = writeln!(text, "## Observed version bumps");
        let _ = writeln!(text);
        let _ = writeln!(text, "| Bump | Records |");
        let _ = writeln!(text, "|---|---:|");
        for bump in bumps {
            let _ = writeln!(
                text,
                "| {} | {} |",
                bump_name(bump),
                self.bumps.get(bump_name(bump)).copied().unwrap_or(0)
            );
        }
        let _ = writeln!(text);

        let assessments = [
            Assessment::Consistent,
            Assessment::ViolationCandidate,
            Assessment::Unknown,
        ];
        let _ = writeln!(text, "## Assessments");
        let _ = writeln!(text);
        let _ = writeln!(text, "| Assessment | Records |");
        let _ = writeln!(text, "|---|---:|");
        for assessment in assessments {
            let _ = writeln!(
                text,
                "| {} | {} |",
                assessment.as_str(),
                self.count_assessment(assessment)
            );
        }
        let _ = writeln!(text);

        let _ = writeln!(text, "## Top rules");
        let _ = writeln!(text);
        if self.rules.is_empty() {
            let _ = writeln!(text, "The analyzed records contain no counted changes.");
        } else {
            let _ = writeln!(text, "| Rule | Counted changes |");
            let _ = writeln!(text, "|---|---:|");
            for (rule, count) in top(&self.rules, TOP_RULES) {
                let _ = writeln!(text, "| {rule} | {count} |");
            }
        }
        let _ = writeln!(text);

        let _ = writeln!(text, "## Skips");
        let _ = writeln!(text);
        if self.skip_reasons.is_empty() {
            let _ = writeln!(text, "The input contains no skipped records.");
        } else {
            let _ = writeln!(text, "| Reason | Packages |");
            let _ = writeln!(text, "|---|---:|");
            for (reason, count) in top(&self.skip_reasons, self.skip_reasons.len()) {
                let _ = writeln!(text, "| {reason} | {count} |");
            }
        }
        text
    }

    /// Count analyzed records with one assessment.
    fn count_assessment(&self, assessment: Assessment) -> usize {
        self.assessments
            .get(assessment.as_str())
            .copied()
            .unwrap_or(0)
    }
}

/// Stable name of a version bump.
fn bump_name(bump: Bump) -> &'static str {
    match bump {
        Bump::Major => "major",
        Bump::Minor => "minor",
        Bump::Other => "other",
        Bump::None => "none",
    }
}

/// One share rendered with one decimal place.
fn share(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "0.0%".to_string();
    }
    format!("{:.1}%", (part as f64 / whole as f64) * PERCENT)
}

/// The largest counts of one histogram, with names breaking ties.
fn top(histogram: &BTreeMap<String, usize>, limit: usize) -> Vec<(&str, usize)> {
    let mut rows: Vec<(&str, usize)> = histogram
        .iter()
        .map(|(name, count)| (name.as_str(), *count))
        .collect();
    rows.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    rows.truncate(limit);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-analyzed, two-skipped JSONL fixture with a violation candidate and
    /// two counted rules.
    const FIXTURE: &str = concat!(
        "{\"schema\":1,\"outcome\":\"analyzed\",\"package\":\"alpha\",\"from\":\"1.0.0\",",
        "\"to\":\"1.1.0\",\"classification\":\"breaking\",\"observedBump\":\"minor\",",
        "\"assessment\":\"violation-candidate\",\"counts\":{\"breaking\":2,\"additive\":0,",
        "\"none\":0,\"unknown\":0,\"internal\":1},\"rules\":{\"pvp/entity-removed\":2},",
        "\"changeCount\":3,\"unknownCount\":0,\"docsSha256\":{\"older\":\"aa\",\"newer\":\"bb\"},",
        "\"diagnostics\":[\"internal-modules-excluded\"]}\n",
        "{\"schema\":1,\"outcome\":\"analyzed\",\"package\":\"beta\",\"from\":\"2.0.0\",",
        "\"to\":\"2.1.0\",\"classification\":\"additive\",\"observedBump\":\"minor\",",
        "\"assessment\":\"consistent\",\"counts\":{\"breaking\":0,\"additive\":3,",
        "\"none\":0,\"unknown\":0,\"internal\":0},\"rules\":{\"pvp/entity-added\":3},",
        "\"changeCount\":3,\"unknownCount\":0,\"docsSha256\":{\"older\":\"cc\",\"newer\":\"dd\"},",
        "\"diagnostics\":[]}\n",
        "{\"schema\":1,\"outcome\":\"skipped\",\"package\":\"gamma\",\"reason\":\"docs\",",
        "\"probed\":[\"1.0.0\"]}\n",
        "{\"schema\":1,\"outcome\":\"skipped\",\"package\":\"delta\",\"reason\":\"fetch\",",
        "\"probed\":[\"2.0.0\",\"1.0.0\"]}\n",
    );

    /// Write the fixture to a temporary file.
    fn fixture(workspace: &Path) -> std::path::PathBuf {
        let path = workspace.join("survey-v1.jsonl");
        fs::write(&path, FIXTURE).expect("write fixture");
        path
    }

    /// Check the totals, the fixed classification, bump, and assessment rows,
    /// the rule order, and the skip reasons of a synthetic file.
    #[test]
    fn aggregates_records() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let input = fixture(workspace.path());
        let text = report(&input, None).expect("report");

        assert!(text.starts_with("# PVP survey report\n"));
        assert!(text.contains("| Records | 4 |"));
        assert!(text.contains("| Analyzed | 2 |"));
        assert!(text.contains("| Skipped | 2 |"));
        assert!(text.contains("| Coverage | 50.0% of records |"));
        assert!(text.contains("| Violation candidates | 1 (50.0% of analyzed) |"));
        assert!(text.contains("| breaking | 1 |"));
        assert!(text.contains("| additive | 1 |"));
        assert!(text.contains("| none | 0 |"));
        assert!(text.contains("| unknown | 0 |"));
        assert!(text.contains("| minor | 2 |"));
        assert!(text.contains("| major | 0 |"));
        assert!(text.contains("| consistent | 1 |"));
        assert!(text.contains("| violation-candidate | 1 |"));
        assert!(text.contains("| docs | 1 |"));
        assert!(text.contains("| fetch | 1 |"));

        // A larger rule count sorts first; equal counts would sort by rule id.
        let added = text.find("| pvp/entity-added | 3 |").expect("added rule");
        let removed = text
            .find("| pvp/entity-removed | 2 |")
            .expect("removed rule");
        assert!(added < removed);
        assert!(text.ends_with('\n'));
    }

    /// Check that the rendered text is stable across calls and that the output
    /// file holds the returned text.
    #[test]
    fn writes_deterministic_markdown() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let input = fixture(workspace.path());
        let first = report(&input, None).expect("first report");
        let second = report(&input, None).expect("second report");
        assert_eq!(first, second);

        let output = workspace.path().join("reports/survey.md");
        let written = report(&input, Some(&output)).expect("written report");
        assert_eq!(written, first);
        assert_eq!(fs::read_to_string(&output).expect("read report"), first);
    }

    /// Check that an empty file renders zero totals without dividing by zero.
    #[test]
    fn renders_empty_input() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let input = workspace.path().join("empty.jsonl");
        fs::write(&input, "").expect("write empty file");
        let text = report(&input, None).expect("empty report");
        assert!(text.contains("| Records | 0 |"));
        assert!(text.contains("| Coverage | 0.0% of records |"));
        assert!(text.contains("The analyzed records contain no counted changes."));
        assert!(text.contains("The input contains no skipped records."));
    }

    /// Check that an empty category list still renders its fixed rows, so a
    /// report over skips only keeps the table shape.
    #[test]
    fn renders_skips_only() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let input = workspace.path().join("skips.jsonl");
        fs::write(
            &input,
            "{\"schema\":1,\"outcome\":\"skipped\",\"package\":\"gamma\",\"reason\":\"docs\",\"probed\":[]}\n",
        )
        .expect("write skips");
        let text = report(&input, None).expect("skips report");
        assert!(text.contains("| Analyzed | 0 |"));
        assert!(text.contains("| docs | 1 |"));
        assert!(text.contains("| breaking | 0 |"));
    }
}
