// src/telemetry/axes.rs
//! The three axes every telemetry decision is made on: where the app runs
//! (Deployment), how much it may send (telemetry level), and how the sender is
//! identified (Attribution).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A value that is not one of an axis's names.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("TA001: '{value}' is not a valid {kind}; expected one of {expected}")]
pub struct ParseAxisError {
    pub kind: &'static str,
    pub value: String,
    pub expected: &'static str,
}

/// Where a derived app runs, which decides the default telemetry level, the
/// sampler, the flush budget and whether the end-user surface exists at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deployment {
    /// Runs on a person's own machine. Telemetry is off until they consent.
    EndUser {
        /// Appended to the telemetry notice as `Details: <url>` when set.
        privacy_url: Option<String>,
    },
    /// Runs as fleet infrastructure the operator already controls.
    Service,
}

impl Default for Deployment {
    fn default() -> Self {
        Self::EndUser { privacy_url: None }
    }
}

impl Deployment {
    pub fn is_end_user(&self) -> bool {
        matches!(self, Self::EndUser { .. })
    }

    pub fn privacy_url(&self) -> Option<&str> {
        match self {
            Self::EndUser { privacy_url } => privacy_url.as_deref(),
            Self::Service => None,
        }
    }

    /// The value recorded as the `cli.deployment` Resource attribute.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EndUser { .. } => "end_user",
            Self::Service => "service",
        }
    }
}

/// How much an Install may send. Ordered: `Off < Usage < Diagnostic < Debug`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TelemetryLevel {
    #[default]
    Off,
    Usage,
    Diagnostic,
    Debug,
}

impl TelemetryLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Usage => "usage",
            Self::Diagnostic => "diagnostic",
            Self::Debug => "debug",
        }
    }

    /// Every telemetry level, lowest first. Used to render help text and the
    /// `telemetry set` argument constraint.
    pub const ALL: [TelemetryLevel; 4] = [Self::Off, Self::Usage, Self::Diagnostic, Self::Debug];
}

impl fmt::Display for TelemetryLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TelemetryLevel {
    type Err = ParseAxisError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(Self::Off),
            "usage" => Ok(Self::Usage),
            "diagnostic" => Ok(Self::Diagnostic),
            "debug" => Ok(Self::Debug),
            other => Err(ParseAxisError {
                kind: "telemetry level",
                value: other.to_string(),
                expected: "off, usage, diagnostic, debug",
            }),
        }
    }
}

/// How the sender of a signal is identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Attribution {
    /// No install id, no session id, no principal.
    Anonymous,
    /// A locally minted install id. The default.
    #[default]
    Pseudonymous,
    /// The app's own identity hook supplied a principal.
    Identified,
}

impl Attribution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::Pseudonymous => "pseudonymous",
            Self::Identified => "identified",
        }
    }
}

impl fmt::Display for Attribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Attribution {
    type Err = ParseAxisError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "anonymous" => Ok(Self::Anonymous),
            "pseudonymous" => Ok(Self::Pseudonymous),
            "identified" => Ok(Self::Identified),
            other => Err(ParseAxisError {
                kind: "attribution",
                value: other.to_string(),
                expected: "anonymous, pseudonymous, identified",
            }),
        }
    }
}
