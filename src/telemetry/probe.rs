// src/telemetry/probe.rs
//! Probes: the named units telemetry is turned on and off by.
//!
//! A probe is `{ id, min_level, summary, sends }`. Ids are dotted and
//! hierarchical: `cli.command.args` is a child of `cli.command`, and a
//! disabled parent disables its whole subtree. Nothing here does I/O, so the
//! rules are testable without a provider, a store or a subscriber.

use super::axes::TelemetryLevel;

/// First segments an application may not claim, because they collide with the
/// non-probe keys of the `telemetry.` configuration subtree.
pub const RESERVED_FIRST_SEGMENTS: &[&str] = &[
    "level",
    "attribution",
    "install_id",
    "notice_shown",
    "endpoint",
    "traces",
    "metrics",
    "logs",
];

/// One probe's declaration. `summary` and `sends` are shown by
/// `telemetry info`, so they are written for a person, not a developer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeSpec {
    pub id: &'static str,
    pub min_level: TelemetryLevel,
    pub summary: &'static str,
    pub sends: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeIdError {
    #[error("PB001: probe id '{0}' is malformed; expected ^[a-z0-9]+(\\.[a-z0-9_]+)*$")]
    Malformed(String),
    #[error("PB002: probe id '{0}' starts with the reserved segment '{1}'")]
    Reserved(String, &'static str),
    #[error("PB003: duplicate probe id '{0}'")]
    Duplicate(String),
}

/// Check an id against `^[a-z0-9]+(\.[a-z0-9_]+)*$` and the reserved list.
///
/// The reserved-segment check runs before the first segment's character-class
/// check: two reserved names (`install_id`, `notice_shown`) contain an
/// underscore, which the first segment's grammar otherwise forbids, and a
/// probe author who types one of those must see `Reserved`, not the generic
/// `Malformed`.
pub fn validate_probe_id(id: &str) -> Result<(), ProbeIdError> {
    let mut segments = id.split('.');
    let first = segments.next().unwrap_or_default();
    if let Some(reserved) = RESERVED_FIRST_SEGMENTS
        .iter()
        .copied()
        .find(|r| *r == first)
    {
        return Err(ProbeIdError::Reserved(id.to_string(), reserved));
    }
    let first_ok = !first.is_empty()
        && first
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    if !first_ok {
        return Err(ProbeIdError::Malformed(id.to_string()));
    }
    for segment in segments {
        let ok = !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !ok {
            return Err(ProbeIdError::Malformed(id.to_string()));
        }
    }
    Ok(())
}

/// Every probe an app knows about: the built-in catalog plus whatever the
/// author registered. Kept sorted by id so `telemetry info` and the generated
/// manifest section are stable.
#[derive(Debug, Clone, Default)]
pub struct ProbeRegistry {
    probes: Vec<ProbeSpec>,
}

impl ProbeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, spec: ProbeSpec) -> Result<(), ProbeIdError> {
        validate_probe_id(spec.id)?;
        if self.probes.iter().any(|p| p.id == spec.id) {
            return Err(ProbeIdError::Duplicate(spec.id.to_string()));
        }
        self.probes.push(spec);
        self.probes.sort_by(|a, b| a.id.cmp(b.id));
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&ProbeSpec> {
        self.probes.iter().find(|p| p.id == id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProbeSpec> {
        self.probes.iter()
    }

    pub fn len(&self) -> usize {
        self.probes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.probes.is_empty()
    }
}

/// `effective(probe) = telemetry level >= probe.min_level && every
/// ancestor-or-self is enabled`. `enabled` answers
/// `telemetry.<probe>.enabled` for one probe id; a probe with no stored value
/// is enabled.
pub fn effective(
    registry: &ProbeRegistry,
    level: TelemetryLevel,
    id: &str,
    enabled: &dyn Fn(&str) -> bool,
) -> bool {
    let Some(spec) = registry.get(id) else {
        return false;
    };
    if level == TelemetryLevel::Off || level < spec.min_level {
        return false;
    }
    let mut prefix = String::with_capacity(id.len());
    for segment in id.split('.') {
        if !prefix.is_empty() {
            prefix.push('.');
        }
        prefix.push_str(segment);
        if registry.contains(&prefix) && !enabled(&prefix) {
            return false;
        }
    }
    true
}

/// What marking a feature does. Split out as a pure function so the release
/// path is testable: the `debug_assert!` on an unregistered name would
/// otherwise make the unregistered case unobservable in a debug build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureOutcome {
    /// Registered: emit the event and the metric with a `feature` label.
    Recorded,
    /// Not registered: emit the event, warn once for this name, never label.
    Unregistered,
}

pub fn feature_outcome(registered: &[&str], name: &str) -> FeatureOutcome {
    if registered.contains(&name) {
        FeatureOutcome::Recorded
    } else {
        FeatureOutcome::Unregistered
    }
}
