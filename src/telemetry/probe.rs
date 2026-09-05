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

use TelemetryLevel::{Debug as Dbg, Diagnostic, Usage};

/// The framework's own probes. An application adds to this list through
/// `AppBuilder::with_telemetry_ops`; it cannot remove from it, but a person or
/// an administrator can disable any of them.
pub const BUILTIN_PROBES: &[ProbeSpec] = &[
    ProbeSpec {
        id: "cli.process",
        min_level: Usage,
        summary: "Process start and exit",
        sends: "That the app ran, its version, and the exit status class",
    },
    ProbeSpec {
        id: "cli.command",
        min_level: Usage,
        summary: "Which command ran",
        sends: "The registered command path, the invocation surface, duration and status",
    },
    ProbeSpec {
        id: "cli.command.args",
        min_level: Diagnostic,
        summary: "Which arguments were supplied",
        sends: "Argument names and how many there were, never their values",
    },
    ProbeSpec {
        id: "cli.command.arg_values",
        min_level: Dbg,
        summary: "Argument values",
        sends: "Values of arguments the author explicitly allowlisted, and no others",
    },
    ProbeSpec {
        id: "cli.usage_error",
        min_level: Usage,
        summary: "Commands that were typed wrongly",
        sends: "The kind of mistake: unknown command, unknown flag, missing argument, \
                invalid value or failed validation",
    },
    ProbeSpec {
        id: "cli.usage_error.token",
        min_level: Dbg,
        summary: "The text that was not understood",
        sends: "The offending token itself",
    },
    ProbeSpec {
        id: "cli.panic",
        min_level: Usage,
        summary: "Crashes",
        sends: "That the app panicked and the source location, never the message",
    },
    ProbeSpec {
        id: "cli.panic.message",
        min_level: Dbg,
        summary: "Crash messages",
        sends: "The panic message text",
    },
    ProbeSpec {
        id: "cli.help",
        min_level: Usage,
        summary: "Help lookups",
        sends: "Which command's help was asked for",
    },
    ProbeSpec {
        id: "cli.feature",
        min_level: Usage,
        summary: "Named features the app marks",
        sends: "The name of a feature the author registered, and nothing else",
    },
    ProbeSpec {
        id: "cli.auth",
        min_level: Usage,
        summary: "Sign-in activity",
        sends: "That a login, logout, refresh or failure happened, never a credential",
    },
    ProbeSpec {
        id: "cli.config",
        min_level: Diagnostic,
        summary: "Configuration reads and writes",
        sends: "Which setting was touched and whether it succeeded, never the value",
    },
    ProbeSpec {
        id: "cli.secrets",
        min_level: Diagnostic,
        summary: "Secret-store activity",
        sends: "Which backend was used and whether it succeeded, never a secret",
    },
    ProbeSpec {
        id: "cli.doctor",
        min_level: Usage,
        summary: "Diagnostic runs",
        sends: "Which checks ran and how severe their findings were",
    },
    ProbeSpec {
        id: "cli.plugin",
        min_level: Diagnostic,
        summary: "Plugin activity",
        sends: "Which plugin loaded or failed to load",
    },
    ProbeSpec {
        id: "cli.chat",
        min_level: Usage,
        summary: "Chat sessions",
        sends: "That a chat session ran and how long it lasted, never prompt text",
    },
    ProbeSpec {
        id: "http.client",
        min_level: Diagnostic,
        summary: "Outbound requests",
        sends: "Method, status and duration, never the URL path or query",
    },
    ProbeSpec {
        id: "http.client.server_address",
        min_level: Diagnostic,
        summary: "Which host was called",
        sends: "The destination host name, on the span only",
    },
    ProbeSpec {
        id: "http.server",
        min_level: Usage,
        summary: "Requests served",
        sends: "The matched route template, method, status and duration",
    },
    ProbeSpec {
        id: "mcp.session",
        min_level: Usage,
        summary: "Agent sessions",
        sends: "That an MCP session ran, which tools it called and how long it lasted",
    },
];

impl ProbeRegistry {
    /// A registry preloaded with [`BUILTIN_PROBES`].
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        for spec in BUILTIN_PROBES {
            registry
                .register(*spec)
                .expect("the built-in probe catalog is valid by construction");
        }
        registry
    }
}
