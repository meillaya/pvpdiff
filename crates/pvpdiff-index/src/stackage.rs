//! Stackage snapshot pins read from a snapshot's `cabal.config`.
//!
//! A snapshot pins one version per package. The blast-radius filter keeps a
//! dependent only at the pinned version, because that is the version a user of
//! the snapshot would build. `installed` entries name packages that ship with
//! GHC and carry no version, so they are skipped.

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::index::IndexError;

const SNAPSHOT_BASE_URL: &str = "https://www.stackage.org";
const CONFIG_FILE: &str = "cabal.config";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const CONSTRAINTS_FIELD: &str = "constraints:";
const ANY_PREFIX: &str = "any.";
const INSTALLED: &str = "installed";
const EXACT_PREFIX: &str = "==";

/// The package versions pinned by one Stackage snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotPins {
    /// Snapshot name, such as `lts-24.67`.
    pub name: String,
    /// Pinned version per package. Boot packages that only say `installed`
    /// carry no version and are absent.
    pub packages: BTreeMap<String, String>,
}

impl SnapshotPins {
    /// Build an empty pin set for a snapshot name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            packages: BTreeMap::new(),
        }
    }

    /// The pinned version for `package`, matched case-insensitively.
    pub fn version_of(&self, package: &str) -> Option<&str> {
        self.packages
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(package))
            .map(|(_, version)| version.as_str())
    }
}

/// URL of a snapshot's `cabal.config`.
pub fn snapshot_url(snapshot: &str) -> Result<String, IndexError> {
    let valid = !snapshot.is_empty()
        && snapshot
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !valid {
        return Err(IndexError::InvalidSnapshot(snapshot.to_string()));
    }
    Ok(format!("{SNAPSHOT_BASE_URL}/{snapshot}/{CONFIG_FILE}"))
}

/// Download and parse one Stackage snapshot's pins.
///
/// `snapshot` is a snapshot name such as `lts-24.67`, or an `http`, `https`,
/// or `file` URL for a snapshot file already at hand.
pub fn fetch_snapshot(snapshot: &str) -> Result<SnapshotPins, IndexError> {
    let url = if has_scheme(snapshot) {
        snapshot.to_string()
    } else {
        snapshot_url(snapshot)?
    };
    let text = fetch_text(&url, snapshot)?;
    parse_snapshot(snapshot, &text)
}

/// Parse a `cabal.config` snapshot body.
pub fn parse_snapshot(name: &str, config: &str) -> Result<SnapshotPins, IndexError> {
    let Some(constraints) = constraints_text(config) else {
        return Err(IndexError::Snapshot {
            snapshot: name.to_string(),
            reason: format!("missing `{CONSTRAINTS_FIELD}` field"),
        });
    };

    let mut packages = BTreeMap::new();
    for entry in constraints.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some((package, version)) = parse_pin(name, entry)? {
            packages.insert(package, version);
        }
    }
    Ok(SnapshotPins {
        name: name.to_string(),
        packages,
    })
}

fn has_scheme(snapshot: &str) -> bool {
    ["http://", "https://", "file://"]
        .iter()
        .any(|prefix| snapshot.starts_with(prefix))
}

fn fetch_text(url: &str, snapshot: &str) -> Result<String, IndexError> {
    let parsed = Url::parse(url)?;
    if parsed.scheme() == "file" {
        let path = parsed
            .to_file_path()
            .map_err(|()| IndexError::UnsupportedUrl(url.to_string()))?;
        return Ok(fs::read_to_string(path)?);
    }
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(IndexError::UnsupportedUrl(url.to_string()));
    }

    let client = Client::builder()
        .user_agent(crate::user_agent())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(FETCH_TIMEOUT)
        .build()?;
    let response = client.get(url).send()?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(IndexError::SnapshotNotFound {
            snapshot: snapshot.to_string(),
        });
    }
    if !response.status().is_success() {
        return Err(IndexError::Status {
            status: response.status().as_u16(),
            url: url.to_string(),
        });
    }
    Ok(response.text()?)
}

/// Collect the `constraints` field body, joining its continuation lines.
///
/// The field ends at the next line that starts in the first column and is not
/// a comment or blank.
fn constraints_text(config: &str) -> Option<String> {
    let mut value = String::new();
    let mut collecting = false;

    for line in config.lines() {
        let trimmed = line.trim();
        // Comments and blank lines are layout, not part of the value.
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }
        let indented = line.chars().next().is_some_and(char::is_whitespace);
        if indented {
            if collecting {
                value.push(' ');
                value.push_str(trimmed);
            }
            continue;
        }
        if collecting {
            break;
        }
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.starts_with(CONSTRAINTS_FIELD) {
            collecting = true;
            value.push_str(&trimmed[CONSTRAINTS_FIELD.len()..]);
        }
    }

    if collecting { Some(value) } else { None }
}

/// Parse one comma separated constraint: `package ==version`, optionally with
/// the older `any.` prefix. `package installed` has no version, so it yields
/// no pin.
fn parse_pin(snapshot: &str, entry: &str) -> Result<Option<(String, String)>, IndexError> {
    let mut fields = entry.split_whitespace();
    let Some(name) = fields.next() else {
        return Ok(None);
    };
    let name = name.strip_prefix(ANY_PREFIX).unwrap_or(name);
    let Some(operator) = fields.next() else {
        return Err(pin_error(snapshot, entry));
    };
    if operator == INSTALLED {
        return Ok(None);
    }
    let Some(version) = operator.strip_prefix(EXACT_PREFIX) else {
        return Err(pin_error(snapshot, entry));
    };
    if fields.next().is_some() || version.is_empty() {
        return Err(pin_error(snapshot, entry));
    }
    if name.is_empty() || pvpdiff_core::HaskellVersion::parse(version).is_err() {
        return Err(pin_error(snapshot, entry));
    }
    Ok(Some((name.to_string(), version.to_string())))
}

fn pin_error(snapshot: &str, entry: &str) -> IndexError {
    IndexError::Snapshot {
        snapshot: snapshot.to_string(),
        reason: format!("unrecognised constraint `{entry}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "\
-- NOTE: Due to revisions, this file may not work.
-- Stackage snapshot from: http://www.stackage.org/snapshot/lts-24.67

with-compiler: ghc-9.10.3
constraints: aeson ==2.2.5.1,
             base installed,
             text ==2.1.3,
             any.bytestring ==0.12.1.0,
             containers ==0.6.7,
             vector ==0.13.1.0,
             text ==2.1.4
";

    /// Check that continuation lines, comments, `installed` entries, and the
    /// legacy `any.` prefix parse into the pinned map, with later duplicates
    /// winning.
    #[test]
    fn parses_snapshot_pins() {
        let pins = parse_snapshot("lts-24.67", CONFIG).expect("pins");
        assert_eq!(pins.name, "lts-24.67");
        assert_eq!(pins.version_of("aeson"), Some("2.2.5.1"));
        assert_eq!(pins.version_of("bytestring"), Some("0.12.1.0"));
        assert_eq!(pins.version_of("text"), Some("2.1.4"));
        assert_eq!(pins.version_of("base"), None);
        assert_eq!(pins.packages.len(), 5);
    }

    /// Check that an indented line inside the field keeps joining entries, and
    /// that a field after the constraints block ends it.
    #[test]
    fn stops_at_next_field() {
        let config = "\
constraints: demo ==1.0.0,
             other ==2.0.0
with-compiler: ghc-9.10.3
";
        let pins = parse_snapshot("nightly", config).expect("pins");
        assert_eq!(pins.packages.len(), 2);
        assert_eq!(pins.version_of("other"), Some("2.0.0"));
    }

    /// Check that a config without a constraints field and a non-numeric pin
    /// are reported as snapshot parse errors.
    #[test]
    fn rejects_malformed_snapshots() {
        assert!(matches!(
            parse_snapshot("lts-24.67", "with-compiler: ghc-9.10.3\n"),
            Err(IndexError::Snapshot { .. })
        ));
        assert!(matches!(
            parse_snapshot("lts-24.67", "constraints: demo ==not-a-version\n"),
            Err(IndexError::Snapshot { .. })
        ));
        assert!(matches!(
            parse_snapshot("lts-24.67", "constraints: demo >=1.0\n"),
            Err(IndexError::Snapshot { .. })
        ));
    }

    /// Check that only plain snapshot names build a stackage.org URL, so a
    /// crafted name cannot reach another host.
    #[test]
    fn builds_snapshot_urls() {
        assert_eq!(
            snapshot_url("lts-24.67").expect("url"),
            "https://www.stackage.org/lts-24.67/cabal.config"
        );
        assert!(matches!(
            snapshot_url("../../etc/passwd"),
            Err(IndexError::InvalidSnapshot(_))
        ));
        assert!(matches!(
            snapshot_url("lts 24.67"),
            Err(IndexError::InvalidSnapshot(_))
        ));
    }

    /// Check that a snapshot file reached through a file URL parses through
    /// the same fetch path the CLI uses.
    #[test]
    fn fetches_snapshot_from_file_url() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("cabal.config");
        fs::write(&path, CONFIG).expect("write fixture");
        let url = Url::from_file_path(&path).expect("file URL");
        let pins = fetch_snapshot(url.as_str()).expect("pins");
        assert_eq!(pins.version_of("containers"), Some("0.6.7"));
        assert_eq!(pins.name, url.as_str());
    }

    /// Check a real Stackage snapshot. Ignored by default because the test
    /// suite runs offline.
    #[test]
    #[ignore]
    fn fetches_live_snapshot() {
        let pins = fetch_snapshot("lts-24.67").expect("live snapshot");
        println!("live snapshot pins: {}", pins.packages.len());
        assert!(pins.packages.len() > 1000);
        assert!(pins.version_of("aeson").is_some());
    }
}
