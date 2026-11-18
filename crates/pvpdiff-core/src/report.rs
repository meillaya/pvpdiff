//! Report model and renderers.

use std::fmt::Write as _;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::classify::{Classification, Confidence, Severity};
use crate::diff::{Change, Counts, Diagnostic, DiffResult, PackageApi};
use crate::version::{
    Assessment, Bump, HaskellVersion, assess, observed_bump, suggest, suggestion_rationale,
};

/// Tool identity embedded in every report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    /// Binary name.
    pub name: String,
    /// Binary version.
    pub version: String,
}

/// Provenance for one downloaded artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactInfo {
    /// Source URL.
    pub url: String,
    /// SHA-256 of the bytes used.
    pub sha256: String,
}

/// One package side as rendered in the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InputInfo {
    /// Version.
    pub version: String,
    /// `known` when a Hoogle database was available.
    pub api_status: String,
    /// Docs artifact provenance.
    pub docs_url: Option<String>,
    /// Docs digest.
    pub docs_sha256: Option<String>,
    /// Cabal artifact URL.
    pub cabal_url: Option<String>,
    /// Cabal digest.
    pub cabal_sha256: Option<String>,
    /// Latest Cabal revision, when present.
    pub cabal_revision: Option<u32>,
}

/// One report input record.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InputRecord {
    /// Artifact kind (`hoogle` or `cabal`).
    pub kind: String,
    /// Package name.
    pub package: String,
    /// Package version.
    pub version: String,
    /// SHA-256 of the bytes.
    pub sha256: String,
}

/// PVP summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PvpSummary {
    /// Observed version bump, or null when versions cannot be parsed.
    pub observed_bump: Option<Bump>,
    /// Assessment against the observed bump.
    pub assessment: Assessment,
    /// Suggested next version.
    pub suggested_version: Option<String>,
    /// Reasons for the assessment.
    pub rationale: Vec<String>,
}

/// Complete diff report, schema version 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Schema version.
    pub schema: u32,
    /// Tool identity.
    pub tool: ToolInfo,
    /// Package name.
    pub package: String,
    /// Old version and provenance.
    pub from: InputInfo,
    /// New version and provenance.
    pub to: InputInfo,
    /// Aggregate classification.
    pub classification: Classification,
    /// Confidence in the aggregate classification.
    pub confidence: Confidence,
    /// Per-class counts.
    pub counts: Counts,
    /// PVP summary.
    pub pvp: PvpSummary,
    /// Sorted changes.
    pub changes: Vec<Change>,
    /// Diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Input digests.
    pub inputs: Vec<InputRecord>,
}

impl Report {
    /// Serialize the report as pretty JSON with a trailing newline.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }

    /// JSON Schema for the report contract.
    pub fn schema_value() -> Value {
        serde_json::to_value(schemars::schema_for!(Report)).unwrap_or(Value::Null)
    }

    /// Human-readable report.
    pub fn to_text(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "pvpdiff {}", self.tool.version);
        let _ = writeln!(
            output,
            "{} {} -> {}",
            self.package, self.from.version, self.to.version
        );
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "Classification: {} ({} confidence)",
            self.classification.as_str(),
            self.confidence.as_str()
        );
        let _ = writeln!(output, "PVP: {}", pvp_line(&self.pvp));
        let _ = writeln!(
            output,
            "Suggested next version: {}",
            self.pvp
                .suggested_version
                .as_deref()
                .unwrap_or("review required")
        );
        let _ = writeln!(
            output,
            "Counts: {} breaking, {} additive, {} none, {} unknown ({} internal not counted)",
            self.counts.breaking,
            self.counts.additive,
            self.counts.none,
            self.counts.unknown,
            self.counts.internal
        );

        for (classification, title) in [
            (Classification::Breaking, "BREAKING"),
            (Classification::Additive, "ADDITIVE"),
            (Classification::Unknown, "UNKNOWN"),
            (Classification::None, "NONE"),
        ] {
            let changes: Vec<&Change> = self
                .changes
                .iter()
                .filter(|change| change.counted && change.classification == classification)
                .collect();
            if changes.is_empty() {
                continue;
            }
            let _ = writeln!(output);
            let _ = writeln!(output, "{title} ({})", changes.len());
            for change in changes {
                let _ = writeln!(output, "  {}", format_change(change));
            }
        }

        let internal: Vec<&Change> = self
            .changes
            .iter()
            .filter(|change| !change.counted)
            .collect();
        if !internal.is_empty() {
            let _ = writeln!(output);
            let _ = writeln!(
                output,
                "INTERNAL MODULES ({}, not counted; use --include-internal or --strict)",
                internal.len()
            );
            for change in internal {
                let _ = writeln!(output, "  {}", format_change(change));
            }
        }

        if !self.diagnostics.is_empty() {
            let _ = writeln!(output);
            let _ = writeln!(output, "DIAGNOSTICS");
            for diagnostic in &self.diagnostics {
                let _ = writeln!(
                    output,
                    "  {}: {} ({})",
                    diagnostic.severity.as_str(),
                    diagnostic.message,
                    diagnostic.code
                );
            }
        }
        output
    }

    /// Markdown changelog skeleton for the analysed package.
    pub fn to_markdown(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "# {} {}", self.package, self.to.version);
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "API diff from `{}` to `{}`.",
            self.from.version, self.to.version
        );
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "- Classification: **{}**",
            self.classification.as_str()
        );
        let _ = writeln!(output, "- PVP: {}", pvp_line(&self.pvp));
        let _ = writeln!(
            output,
            "- Suggested version: `{}`",
            self.pvp
                .suggested_version
                .as_deref()
                .unwrap_or("review required")
        );
        let _ = writeln!(
            output,
            "- Counts: {} breaking, {} additive, {} unknown",
            self.counts.breaking, self.counts.additive, self.counts.unknown
        );

        for (classification, title) in [
            (Classification::Breaking, "Breaking"),
            (Classification::Additive, "Additive"),
            (Classification::Unknown, "Unknown: review"),
        ] {
            let changes: Vec<&Change> = self
                .changes
                .iter()
                .filter(|change| change.counted && change.classification == classification)
                .collect();
            if changes.is_empty() {
                continue;
            }
            let _ = writeln!(output);
            let _ = writeln!(output, "## {title}");
            let _ = writeln!(output);
            for change in changes {
                let _ = writeln!(output, "- {}", format_change(change));
            }
        }

        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "Generated by pvpdiff {} from Hackage data. Behaviour changes are not visible in this report.",
            self.tool.version
        );
        output
    }
}

/// Artifact provenance for one side of a report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideArtifacts {
    /// Hoogle database provenance.
    pub docs: Option<ArtifactInfo>,
    /// Cabal file provenance.
    pub cabal: Option<ArtifactInfo>,
}

/// Build a report from two package sides and a diff.
pub fn build_report(
    old: &PackageApi,
    old_artifacts: SideArtifacts,
    new: &PackageApi,
    new_artifacts: SideArtifacts,
    result: &DiffResult,
    mut diagnostics: Vec<Diagnostic>,
) -> Report {
    let SideArtifacts {
        docs: old_docs,
        cabal: old_cabal,
    } = old_artifacts;
    let SideArtifacts {
        docs: new_docs,
        cabal: new_cabal,
    } = new_artifacts;
    diagnostics.extend(result.diagnostics.clone());
    diagnostics.sort();
    diagnostics.dedup();

    let classification = result.classification();
    let confidence = confidence_of(result, classification);
    let counts = result.counts();
    let from = input_info(
        &old.surface,
        old_docs,
        old_cabal,
        old.cabal.as_ref().and_then(|cabal| cabal.revision),
    );
    let to = input_info(
        &new.surface,
        new_docs,
        new_cabal,
        new.cabal.as_ref().and_then(|cabal| cabal.revision),
    );
    let pvp = pvp_summary(&old.surface.version, &new.surface.version, classification);

    let mut inputs = Vec::new();
    for (side, info) in [(&old.surface, &from), (&new.surface, &to)] {
        if let (Some(url), Some(sha256)) = (&info.docs_url, &info.docs_sha256) {
            let _ = url;
            inputs.push(InputRecord {
                kind: "hoogle".to_string(),
                package: side.package.clone(),
                version: side.version.clone(),
                sha256: sha256.clone(),
            });
        }
        if let Some(sha256) = &info.cabal_sha256 {
            inputs.push(InputRecord {
                kind: "cabal".to_string(),
                package: side.package.clone(),
                version: side.version.clone(),
                sha256: sha256.clone(),
            });
        }
    }
    inputs.sort();

    Report {
        schema: 1,
        tool: ToolInfo {
            name: "pvpdiff".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        package: old.surface.package.clone(),
        from,
        to,
        classification,
        confidence,
        counts,
        pvp,
        changes: result.changes.clone(),
        diagnostics,
        inputs,
    }
}

fn input_info(
    surface: &pvpdiff_parse::ApiSurface,
    docs: Option<ArtifactInfo>,
    cabal: Option<ArtifactInfo>,
    cabal_revision: Option<u32>,
) -> InputInfo {
    InputInfo {
        version: surface.version.clone(),
        api_status: if docs.is_some() {
            "known".to_string()
        } else {
            "unknown".to_string()
        },
        docs_url: docs.as_ref().map(|artifact| artifact.url.clone()),
        docs_sha256: docs.as_ref().map(|artifact| artifact.sha256.clone()),
        cabal_url: cabal.as_ref().map(|artifact| artifact.url.clone()),
        cabal_sha256: cabal.as_ref().map(|artifact| artifact.sha256.clone()),
        cabal_revision,
    }
}

/// Build a report when a Hoogle database is unavailable and no diff is
/// possible. The result is `unknown`, with the supplied diagnostics.
pub fn unavailable_report(
    package: &str,
    from_version: &str,
    to_version: &str,
    diagnostics: Vec<Diagnostic>,
) -> Report {
    Report {
        schema: 1,
        tool: ToolInfo {
            name: "pvpdiff".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        package: package.to_string(),
        from: InputInfo {
            version: from_version.to_string(),
            api_status: "unknown".to_string(),
            docs_url: None,
            docs_sha256: None,
            cabal_url: None,
            cabal_sha256: None,
            cabal_revision: None,
        },
        to: InputInfo {
            version: to_version.to_string(),
            api_status: "unknown".to_string(),
            docs_url: None,
            docs_sha256: None,
            cabal_url: None,
            cabal_sha256: None,
            cabal_revision: None,
        },
        classification: Classification::Unknown,
        confidence: Confidence::Low,
        counts: Counts::default(),
        pvp: PvpSummary {
            observed_bump: None,
            assessment: Assessment::Unknown,
            suggested_version: None,
            rationale: vec!["a Hoogle database is unavailable".to_string()],
        },
        changes: Vec::new(),
        diagnostics,
        inputs: Vec::new(),
    }
}

fn confidence_of(result: &DiffResult, classification: Classification) -> Confidence {
    result
        .changes
        .iter()
        .filter(|change| change.counted && change.classification == classification)
        .map(|change| change.confidence)
        .min()
        .unwrap_or(Confidence::High)
}

fn pvp_summary(from: &str, to: &str, classification: Classification) -> PvpSummary {
    let Ok(from_version) = HaskellVersion::parse(from) else {
        return PvpSummary {
            observed_bump: None,
            assessment: Assessment::Unknown,
            suggested_version: None,
            rationale: vec!["cannot parse the old version".to_string()],
        };
    };
    let Ok(to_version) = HaskellVersion::parse(to) else {
        return PvpSummary {
            observed_bump: None,
            assessment: Assessment::Unknown,
            suggested_version: None,
            rationale: vec!["cannot parse the new version".to_string()],
        };
    };
    let bump = observed_bump(&from_version, &to_version);
    let assessment = assess(classification, bump);
    let suggested = suggest(&from_version, classification).map(|version| version.to_string());
    let mut rationale = vec![suggestion_rationale(classification).to_string()];
    rationale.push(format!("observed version change: {}", bump_text(bump)));
    if assessment == Assessment::ViolationCandidate {
        rationale.push(format!(
            "the observed {} bump is smaller than the classification requires",
            bump_text(bump)
        ));
    }
    PvpSummary {
        observed_bump: Some(bump),
        assessment,
        suggested_version: suggested,
        rationale,
    }
}

fn bump_text(bump: Bump) -> &'static str {
    match bump {
        Bump::Major => "major",
        Bump::Minor => "minor",
        Bump::Other => "other-component",
        Bump::None => "none",
    }
}

fn pvp_line(pvp: &PvpSummary) -> String {
    let observed = pvp.observed_bump.map(bump_text).unwrap_or("unknown");
    format!(
        "observed {observed} bump; assessment {}",
        pvp.assessment.as_str()
    )
}

fn format_change(change: &Change) -> String {
    let target = change
        .modules
        .first()
        .map(String::as_str)
        .unwrap_or("package");
    let name = change.name.as_deref().unwrap_or("");
    let prefix = match change.classification {
        Classification::Breaking => "-",
        Classification::Additive => "+",
        Classification::Unknown => "?",
        Classification::None => ".",
    };
    let mut line = if matches!(change.kind.as_str(), "module" | "reexport") {
        format!("{prefix} {target}")
    } else {
        format!("{prefix} {target}.{name}")
    };
    match (&change.before, &change.after) {
        (Some(before), Some(after)) => {
            let _ = write!(line, "  {before} -> {after}");
        }
        (Some(before), None) => {
            let _ = write!(line, "  {before}");
        }
        (None, Some(after)) => {
            let _ = write!(line, "  {after}");
        }
        (None, None) => {}
    }
    let _ = write!(line, "  ({})", change.message);
    line
}

/// Format one diagnostic for human output.
pub fn diagnostic_text(diagnostic: &Diagnostic) -> String {
    format!(
        "{}: {} ({})",
        match diagnostic.severity {
            Severity::Warning => "warning",
            Severity::Info => "info",
        },
        diagnostic.message,
        diagnostic.code
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pvpdiff_parse::parse_hoogle;

    fn package(version: &str, body: &str) -> PackageApi {
        let database = format!(
            "-- Hoogle documentation, generated by Haddock\n-- See Hoogle, http://www.haskell.org/hoogle/\n\n@package demo\n@version {version}\n\nmodule Demo\n\n{body}\n"
        );
        PackageApi {
            surface: parse_hoogle(&database).expect("parse"),
            cabal: None,
        }
    }

    /// Check that a report renders a consistent PVP summary and JSON with the
    /// schema field.
    #[test]
    fn builds_and_renders_report() {
        let old = package("1.0.0", "foo :: Int");
        let new = package("1.1.0", "foo :: Int\nbar :: Int");
        let result = crate::diff::diff(&old, &new, crate::diff::DiffOptions::default());
        let report = build_report(
            &old,
            SideArtifacts::default(),
            &new,
            SideArtifacts::default(),
            &result,
            Vec::new(),
        );
        assert_eq!(report.classification, Classification::Additive);
        assert_eq!(report.pvp.assessment, Assessment::Consistent);
        assert_eq!(report.pvp.suggested_version.as_deref(), Some("1.0.1"));
        let json = report.to_json().expect("json");
        assert!(json.contains("\"schema\": 1"));
        assert!(report.to_text().contains("ADDITIVE"));
        assert!(report.to_markdown().contains("## Additive"));
    }

    /// Check that an unknown classification has no suggested version.
    #[test]
    fn unknown_classification_has_no_suggestion() {
        let old = package("1.0.0", "foo :: Int");
        let new = package("1.0.1", "foo :: String");
        let mut result = crate::diff::diff(&old, &new, crate::diff::DiffOptions::default());
        for change in &mut result.changes {
            change.classification = Classification::Unknown;
        }
        let report = build_report(
            &old,
            SideArtifacts::default(),
            &new,
            SideArtifacts::default(),
            &result,
            Vec::new(),
        );
        assert!(report.pvp.suggested_version.is_none());
        assert_eq!(report.pvp.assessment, Assessment::Unknown);
    }
}
