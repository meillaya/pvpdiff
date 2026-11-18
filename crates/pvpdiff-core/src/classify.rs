//! Classification and confidence values shared by the rule engine and reports.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Aggregate or per-change PVP classification.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    /// A breaking change candidate; a major version bump is required.
    Breaking,
    /// Additions only; a minor version bump is required.
    Additive,
    /// No public API change.
    None,
    /// The data does not support a decision.
    Unknown,
}

impl Classification {
    /// Stable lowercase name used in JSON and SARIF.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Breaking => "breaking",
            Self::Additive => "additive",
            Self::None => "none",
            Self::Unknown => "unknown",
        }
    }

    /// The higher severity of two classifications. Breaking wins, then
    /// unknown, then additive, then none.
    pub fn max(self, other: Self) -> Self {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Breaking => 4,
            Self::Unknown => 3,
            Self::Additive => 2,
            Self::None => 1,
        }
    }
}

/// Confidence in one classification.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// The data directly supports the classification.
    High,
    /// The classification is likely but depends on a heuristic.
    Medium,
    /// The classification is a fallback.
    Low,
}

impl Confidence {
    /// Stable lowercase name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// Severity of a diagnostic.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The analysis is incomplete or a decision is unsafe.
    Warning,
    /// Useful context only.
    Info,
}

impl Severity {
    /// Stable lowercase name used in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}
