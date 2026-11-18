//! Haskell version numbers and PVP bump arithmetic.
//!
//! A version is a list of numeric components. Ordering follows Cabal, not
//! SemVer: components compare left to right, and a longer list wins when the
//! shared prefix is equal (`2.0.1.0 > 2.0.1`). The PVP defines the first two
//! components as the major version and the third as the minor version; extra
//! components are unregulated.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::classify::Classification;

/// A parsed Haskell package version.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct HaskellVersion {
    components: Vec<u64>,
}

impl HaskellVersion {
    /// Parse a dot-separated numeric version.
    pub fn parse(text: &str) -> Result<Self, VersionError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(VersionError::Empty);
        }
        let mut components = Vec::new();
        for part in trimmed.split('.') {
            if part.is_empty() {
                return Err(VersionError::BadComponent(trimmed.to_string()));
            }
            let component = part
                .parse::<u64>()
                .map_err(|_| VersionError::BadComponent(trimmed.to_string()))?;
            components.push(component);
        }
        if components.is_empty() {
            return Err(VersionError::Empty);
        }
        Ok(Self { components })
    }

    /// Major version pair `(A, B)`. Missing components are zero.
    pub fn major(&self) -> (u64, u64) {
        (
            self.components.first().copied().unwrap_or(0),
            self.components.get(1).copied().unwrap_or(0),
        )
    }

    /// Minor version `C`. A missing component is zero.
    pub fn minor(&self) -> u64 {
        self.components.get(2).copied().unwrap_or(0)
    }

    /// Number of components in the source version.
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Whether the version has no components. Parsing never produces this.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Components after the first three.
    pub fn extras(&self) -> &[u64] {
        self.components.get(3..).unwrap_or(&[])
    }
}

impl fmt::Display for HaskellVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for component in &self.components {
            if !first {
                write!(formatter, ".")?;
            }
            write!(formatter, "{component}")?;
            first = false;
        }
        Ok(())
    }
}

/// Failure to parse a version.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VersionError {
    /// The input was empty.
    #[error("version is empty")]
    Empty,
    /// A component was not a non-negative integer.
    #[error("version has a non-numeric component: {0}")]
    BadComponent(String),
}

/// How the version number changed between two releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Bump {
    /// The major version `A.B` changed.
    Major,
    /// The minor version `C` changed.
    Minor,
    /// Only extra components changed, or components were appended.
    Other,
    /// The versions are equal.
    None,
}

/// Result of comparing a change classification with the observed version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Assessment {
    /// The observed bump does not contradict the classification.
    Consistent,
    /// The classification requires a larger bump than observed.
    ViolationCandidate,
    /// The data does not support an assessment.
    Unknown,
}

impl Assessment {
    /// Stable name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Consistent => "consistent",
            Self::ViolationCandidate => "violation-candidate",
            Self::Unknown => "unknown",
        }
    }
}

/// Compare two versions and return the observed PVP bump.
pub fn observed_bump(from: &HaskellVersion, to: &HaskellVersion) -> Bump {
    if from == to {
        return Bump::None;
    }
    if from.major() != to.major() {
        return Bump::Major;
    }
    if from.minor() != to.minor() {
        return Bump::Minor;
    }
    Bump::Other
}

/// Assess the observed bump against a change classification.
pub fn assess(classification: Classification, bump: Bump) -> Assessment {
    match classification {
        Classification::Breaking => match bump {
            Bump::Major => Assessment::Consistent,
            Bump::Minor | Bump::Other | Bump::None => Assessment::ViolationCandidate,
        },
        Classification::Additive => match bump {
            Bump::Major | Bump::Minor => Assessment::Consistent,
            Bump::Other | Bump::None => Assessment::ViolationCandidate,
        },
        Classification::None => Assessment::Consistent,
        Classification::Unknown => Assessment::Unknown,
    }
}

/// Suggest the next version for a classification.
///
/// Breaking changes suggest `A.(B+1).0`; additive changes suggest
/// `A.B.(C+1)`; a clean diff keeps the current version. Unknown results have no
/// suggestion. Extra components are unregulated by the PVP and are dropped.
pub fn suggest(from: &HaskellVersion, classification: Classification) -> Option<HaskellVersion> {
    let (major, minor) = from.major();
    match classification {
        Classification::Breaking => Some(HaskellVersion {
            components: vec![major, minor + 1, 0],
        }),
        Classification::Additive => Some(HaskellVersion {
            components: vec![major, minor, from.minor() + 1],
        }),
        Classification::None => Some(from.clone()),
        Classification::Unknown => None,
    }
}

/// Human-readable rationale for a suggested version.
pub fn suggestion_rationale(classification: Classification) -> &'static str {
    match classification {
        Classification::Breaking => {
            "breaking changes require a greater A.B; the suggestion resets C"
        }
        Classification::Additive => "additions require a greater C; the suggestion keeps A.B",
        Classification::None => "no public API change; no bump is required",
        Classification::Unknown => "review the unknown changes before choosing a version",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Check that Cabal ordering treats a longer list as greater when the
    /// shared prefix is equal.
    #[test]
    fn orders_versions_like_cabal() {
        let short = HaskellVersion::parse("2.0.1").expect("version");
        let long = HaskellVersion::parse("2.0.1.0").expect("version");
        assert!(long > short);
        assert_ne!(long, short);
    }

    /// Check the major, minor, other, and none bump cases.
    #[test]
    fn classifies_observed_bumps() {
        let one = HaskellVersion::parse("1.2.3").expect("version");
        assert_eq!(
            observed_bump(&one, &HaskellVersion::parse("2.0.0").expect("version")),
            Bump::Major
        );
        assert_eq!(
            observed_bump(&one, &HaskellVersion::parse("1.3.0").expect("version")),
            Bump::Major
        );
        assert_eq!(
            observed_bump(&one, &HaskellVersion::parse("1.2.4").expect("version")),
            Bump::Minor
        );
        assert_eq!(
            observed_bump(&one, &HaskellVersion::parse("1.2.3.1").expect("version")),
            Bump::Other
        );
        assert_eq!(observed_bump(&one, &one), Bump::None);
    }

    /// Check that breaking and additive suggestions use the PVP components.
    #[test]
    fn suggests_pvp_versions() {
        let version = HaskellVersion::parse("1.2.3.4").expect("version");
        assert_eq!(
            suggest(&version, Classification::Breaking)
                .expect("suggestion")
                .to_string(),
            "1.3.0"
        );
        assert_eq!(
            suggest(&version, Classification::Additive)
                .expect("suggestion")
                .to_string(),
            "1.2.4"
        );
        assert!(suggest(&version, Classification::Unknown).is_none());
    }

    /// Check the assessment matrix for breaking, additive, and unknown changes.
    #[test]
    fn assesses_against_bumps() {
        assert_eq!(
            assess(Classification::Breaking, Bump::Minor),
            Assessment::ViolationCandidate
        );
        assert_eq!(
            assess(Classification::Breaking, Bump::Major),
            Assessment::Consistent
        );
        assert_eq!(
            assess(Classification::Additive, Bump::Minor),
            Assessment::Consistent
        );
        assert_eq!(
            assess(Classification::Additive, Bump::None),
            Assessment::ViolationCandidate
        );
        assert_eq!(
            assess(Classification::Unknown, Bump::Minor),
            Assessment::Unknown
        );
    }
}
