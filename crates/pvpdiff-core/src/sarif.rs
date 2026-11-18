//! SARIF 2.1.0 rendering for GitHub code scanning.
//!
//! The document is built as JSON and deserialized into `serde-sarif` types
//! before serialization. That catches shape errors in tests while keeping the
//! renderer source small and deterministic. No invocation timestamps are
//! emitted.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::classify::Classification;
use crate::diff::Change;
use crate::report::Report;
use crate::rules;

/// SARIF schema URI accepted by GitHub code scanning.
const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
const SARIF_VERSION: &str = "2.1.0";
const FINGERPRINT_KEY: &str = "pvpdiff/v1";

/// Render a report as SARIF 2.1.0.
///
/// `artifact_uri` is a repository-relative path used for every result
/// location. The Hoogle data has no source file or line, so results point at
/// the Cabal file (or the caller-provided path).
pub fn render(
    report: &Report,
    artifact_uri: &str,
    include_internal: bool,
) -> Result<String, serde_json::Error> {
    let changes: Vec<&Change> = report
        .changes
        .iter()
        .filter(|change| change.counted || include_internal)
        .collect();

    let mut used_rules: BTreeMap<&str, usize> = BTreeMap::new();
    for change in &changes {
        let index = used_rules.len();
        used_rules.entry(change.rule.as_str()).or_insert(index);
    }
    let rule_documents: Vec<Value> = used_rules
        .keys()
        .map(|id| {
            let summary = rules::rule(id).map(|rule| rule.summary).unwrap_or(id);
            json!({
                "id": id,
                "name": id,
                "shortDescription": { "text": summary },
                "fullDescription": { "text": summary },
                "help": { "text": format!("{id}: {summary}") },
            })
        })
        .collect();
    let results: Vec<Value> = changes
        .iter()
        .map(|change| {
            let rule_index = used_rules.get(change.rule.as_str()).copied().unwrap_or(0);
            json!({
                "ruleId": change.rule,
                "ruleIndex": rule_index,
                "level": level(change.classification),
                "message": { "text": message(change) },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": artifact_uri },
                        "region": { "startLine": 1, "startColumn": 1 }
                    }
                }],
                "partialFingerprints": {
                    (FINGERPRINT_KEY): fingerprint(change)
                },
                "properties": {
                    "classification": change.classification.as_str(),
                    "modules": change.modules,
                    "name": change.name,
                    "counted": change.counted
                }
            })
        })
        .collect();

    let document = json!({
        "$schema": SARIF_SCHEMA,
        "version": SARIF_VERSION,
        "runs": [{
            "tool": {
                "driver": {
                    "name": "pvpdiff",
                    "version": report.tool.version,
                    "semanticVersion": report.tool.version,
                    "informationUri": "https://github.com/meillaya/pvpdiff",
                    "rules": rule_documents,
                }
            },
            "results": results,
            "properties": {
                "package": report.package,
                "from": report.from.version,
                "to": report.to.version,
                "classification": report.classification.as_str(),
                "assessment": report.pvp.assessment.as_str()
            }
        }]
    });

    // Deserializing into the typed schema catches a malformed document before
    // a consumer sees it.
    let typed: serde_sarif::sarif::Sarif = serde_json::from_value(document.clone())?;
    let mut text = serde_json::to_string_pretty(&typed)?;
    text.push('\n');
    Ok(text)
}

fn level(classification: Classification) -> &'static str {
    match classification {
        Classification::Breaking => "error",
        Classification::Unknown => "warning",
        Classification::Additive | Classification::None => "note",
    }
}

fn message(change: &Change) -> String {
    let target = change
        .modules
        .first()
        .map(String::as_str)
        .unwrap_or("package");
    let name = change.name.as_deref().unwrap_or("");
    let mut text = format!(
        "{} {} {}: {}",
        change.classification.as_str(),
        change.rule,
        if name.is_empty() {
            target.to_string()
        } else {
            format!("{target}.{name}")
        },
        change.message
    );
    if let Some(before) = &change.before {
        text.push_str("; before: ");
        text.push_str(before);
    }
    if let Some(after) = &change.after {
        text.push_str("; after: ");
        text.push_str(after);
    }
    text
}

fn fingerprint(change: &Change) -> String {
    let key = format!(
        "{}|{}|{}|{}|{}",
        change.rule,
        change.modules.join(","),
        change.name.as_deref().unwrap_or(""),
        change.before.as_deref().unwrap_or(""),
        change.after.as_deref().unwrap_or("")
    );
    blake3::hash(key.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pvpdiff_parse::parse_hoogle;

    fn package(version: &str, body: &str) -> crate::diff::PackageApi {
        let database = format!(
            "-- Hoogle documentation, generated by Haddock\n-- See Hoogle, http://www.haskell.org/hoogle/\n\n@package demo\n@version {version}\n\nmodule Demo\n\n{body}\n"
        );
        crate::diff::PackageApi {
            surface: parse_hoogle(&database).expect("parse"),
            cabal: None,
        }
    }

    /// Check that a report renders valid SARIF with rules, results, levels,
    /// locations, and deterministic fingerprints.
    #[test]
    fn renders_valid_sarif() {
        let old = package("1.0.0", "foo :: Int");
        let new = package("1.1.0", "bar :: Int");
        let diff = crate::diff::diff(&old, &new, crate::diff::DiffOptions::default());
        let report = crate::report::build_report(
            &old,
            crate::report::SideArtifacts::default(),
            &new,
            crate::report::SideArtifacts::default(),
            &diff,
            Vec::new(),
        );
        let text = render(&report, "demo.cabal", false).expect("sarif");
        let parsed: serde_sarif::sarif::Sarif = serde_json::from_str(&text).expect("parse sarif");
        assert_eq!(parsed.version.to_string(), "\"2.1.0\"");
        assert_eq!(parsed.runs[0].tool.driver.name, "pvpdiff");
        assert!(
            parsed.runs[0]
                .results
                .as_ref()
                .is_some_and(|results| !results.is_empty())
        );
        let results = parsed.runs[0].results.as_ref().expect("results");
        let error = results
            .iter()
            .find(|result| result.level == Some(serde_sarif::sarif::ResultLevel::Error))
            .expect("breaking result");
        assert!(error.locations.as_ref().is_some_and(|locations| {
            locations[0]
                .physical_location
                .as_ref()
                .and_then(|location| location.artifact_location.as_ref())
                .and_then(|artifact| artifact.uri.as_deref())
                == Some("demo.cabal")
        }));
        assert!(
            error
                .partial_fingerprints
                .as_ref()
                .is_some_and(|fingerprints| fingerprints.contains_key(FINGERPRINT_KEY))
        );
    }
}
