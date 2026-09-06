// src/telemetry/commands.rs
//! The built-in `telemetry` command group: `status`, `set`, `info`,
//! `disable`, `enable`, `reset` (spec 025, "The end-user surface").
//!
//! Only registered for [`Deployment::EndUser`] apps (see the guard in
//! `AppBuilder::build`). On a server deployment the telemetry level is an
//! operator's configuration decision made through whatever config surface the
//! application already has — a `telemetry` command giving an end user (or a
//! process on the same host) a second, competing way to flip it would be a
//! second source of truth for the same setting, so the command group simply
//! does not exist there.
//!
//! Follows the exact `Command`/`CommandSpec`/`CommandPath`/`GroupMetadata`
//! construction style of [`crate::config::commands`] (`config show`/
//! `manifest`/`profile`/`refresh`): no `#[derive(CommandSpec)]`, hand-built
//! leaves registered under one group. Unlike that module, every leaf here
//! needs its own [`TelemetryStore`], opened fresh on each invocation through
//! a captured [`TelemetryStoreLocation`] rather than through any
//! `AppContext` accessor — telemetry settings are deliberately not part of
//! the application's own config backend (see `store.rs`'s module doc), and a
//! test needs to point every command at an isolated temporary directory
//! without mutating global process state (`XDG_CONFIG_HOME` or similar),
//! which is exactly what threading a `TelemetryStoreLocation` through the
//! closure buys.
//!
//! The pure functions below (`status_report`, `set_level`, `disable_probe`,
//! `enable_probe`, `reset`) never touch `AppContext` or `DiagnosticReporter`:
//! they take a `&TelemetryStore` (and sometimes a `&ProbeRegistry`) and
//! return a typed `Result`. That is what `tests/unit/telemetry_commands.rs`
//! exercises directly. The `build_*_command` functions below are the thin
//! CLI-facing layer that calls them and turns the typed error into a
//! `Diagnostic` a person actually sees — that seam is what
//! `tests/integration/telemetry_cli.rs` exercises end-to-end through a real
//! `AppBuilder` + `CliTestHarness`.

use crate::app::diagnostic_reporter::DiagnosticReporter;
use crate::command::{Command, CommandRegistry};
use crate::config::resolution::Layer;
use crate::config::ConfigError;
use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::{TEL001, TEL002, TEL003};
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandPath, CommandSpec, ExitCodeEntry, GroupMetadata};
use crate::spec::value::ArgValue;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use super::axes::{Deployment, TelemetryLevel};
use super::policy::{
    detect_kill_switch, resolve_policy, LayeredLevel, TelemetryInputs, TelemetryPolicy,
};
use super::probe::ProbeRegistry;
use super::store::{StoreState, TelemetryStore, TelemetryStoreLocation};

/// One probe's state as reported by `telemetry status`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProbeStatus {
    pub id: String,
    pub min_level: String,
    /// Whether this exact probe id has been disabled (directly, not by an
    /// ancestor). Absent from the store means enabled.
    pub enabled: bool,
    /// Whether the probe actually fires right now — false if the level is
    /// too low, if this probe was disabled, or if any ancestor was.
    pub effective: bool,
    pub summary: String,
}

/// `telemetry status`'s full report, and the exact shape of `telemetry
/// status --json`'s stdout. Every field always serializes (no
/// `skip_serializing_if`) — an anonymous install still has an `install_id`
/// key, it is just `null`, so a script reading the JSON never has to
/// special-case a missing key.
#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub level: String,
    pub level_source: String,
    pub attribution: String,
    pub install_id: Option<String>,
    pub endpoint: Option<String>,
    pub endpoint_source: Option<String>,
    pub policy: String,
    pub kill_switch: Option<String>,
    pub probes: Vec<ProbeStatus>,
    pub store: String,
}

/// The outcome of `telemetry set`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetOutcome {
    /// The level was written and is in effect.
    Applied,
    /// The level was written, but a kill switch clamps the *effective*
    /// level below it — the write still succeeded, so `telemetry status`
    /// keeps reporting the configured level once the kill switch is lifted.
    AppliedButClamped {
        effective: TelemetryLevel,
        reason: String,
    },
}

/// Errors from the pure `telemetry` command functions. The CLI layer below
/// turns these into a `Diagnostic` a person reads; a caller driving these
/// functions directly (as the unit tests do) gets a typed, matchable error
/// instead of a formatted string.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryCommandError {
    /// The settings file could not be written. Carries the store's own
    /// reason verbatim (not just "unavailable") — a person reading this
    /// needs the actual cause, not a category.
    #[error("telemetry store unavailable: {0}")]
    StoreUnavailable(String),
    #[error("unknown probe {0:?}; run `telemetry info` to see the full catalog")]
    UnknownProbe(String),
    #[error(transparent)]
    Write(#[from] ConfigError),
}

fn map_store_error(err: ConfigError) -> TelemetryCommandError {
    match err {
        ConfigError::ReadOnly { backend } => TelemetryCommandError::StoreUnavailable(backend),
        other => TelemetryCommandError::Write(other),
    }
}

// ── shared rendering helpers ─────────────────────────────────────────────────

/// A [`Layer`]'s wire label. Verbatim copy of
/// `crate::config::commands::layer_label` — that one is private to its own
/// module, and telemetry's status report wants the exact same
/// `snake_case` rendering (`config_file`, not `ConfigFile`) for its own
/// `level_source`/`endpoint_source` fields.
fn layer_label(layer: Layer) -> String {
    serde_json::to_value(layer)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// One line describing whether an organisation governs this setting.
///
/// Simplified relative to the full PRD picture: `TelemetryPolicy` carries no
/// dedicated "is this managed" flag today, only `level_source`, so this
/// reads `Layer::Enforced` as "enforced" and everything else as "not
/// managed". A real `PolicyClient`-backed profile/version (as `config
/// profile` reports) is future work — there is no such client wired into
/// the telemetry store yet.
fn policy_line(policy: &TelemetryPolicy) -> String {
    if policy.level_source == Layer::Enforced {
        "organisation policy: enforced".to_string()
    } else {
        "organisation policy: not managed".to_string()
    }
}

/// Build a `TelemetryPolicy` from what is currently on disk. There is no
/// startup-time `TelemetryPolicy` to reuse here (PR7 owns that wiring) — each
/// leaf command derives its own from a fresh read, which is correct for a
/// short-lived CLI invocation: the whole point of `telemetry status` is to
/// show the *current* on-disk state, not a cached one.
fn build_policy(app_name: &str, store: &TelemetryStore) -> TelemetryPolicy {
    let settings = store.settings();

    let mut disabled_probes = BTreeSet::new();
    for (id, enabled) in &settings.probes {
        if !*enabled {
            disabled_probes.insert(id.clone());
        }
    }

    let kill_switch = detect_kill_switch(app_name, &|k| std::env::var(k).ok());
    let endpoint_source = settings.endpoint.as_ref().map(|_| Layer::ConfigFile);

    let inputs = TelemetryInputs {
        app: app_name.to_string(),
        deployment: Deployment::EndUser { privacy_url: None },
        level: LayeredLevel {
            config_file: settings.level,
            ..Default::default()
        },
        endpoint: settings.endpoint.clone(),
        endpoint_source,
        attribution: settings.attribution.unwrap_or_default(),
        install_id: settings.install_id.clone(),
        registry: ProbeRegistry::with_builtins(),
        disabled_probes,
        store_available: store.state().is_ready(),
        store_error: store.state().reason().map(str::to_string),
        kill_switch,
        ..Default::default()
    };
    resolve_policy(inputs)
}

// ── pure command functions (unit-tested directly) ──────────────────────────

/// Render `policy`'s current state as a `telemetry status` report.
pub fn status_report(policy: &TelemetryPolicy, store: &StoreState) -> StatusReport {
    let probes: Vec<ProbeStatus> = policy
        .registry
        .iter()
        .map(|probe| ProbeStatus {
            id: probe.id.to_string(),
            min_level: probe.min_level.as_str().to_string(),
            enabled: !policy.disabled_probes.contains(probe.id),
            effective: policy.effective(probe.id),
            summary: probe.summary.to_string(),
        })
        .collect();

    StatusReport {
        level: policy.level.as_str().to_string(),
        level_source: layer_label(policy.level_source),
        attribution: policy.attribution.as_str().to_string(),
        install_id: policy.install_id.clone(),
        endpoint: policy.endpoint.clone(),
        endpoint_source: policy.endpoint_source.map(layer_label),
        policy: policy_line(policy),
        kill_switch: policy.kill_switch.map(|ks| ks.env_var(&policy.app)),
        probes,
        store: store.describe(),
    }
}

/// Write a new telemetry level. Returns [`SetOutcome::AppliedButClamped`]
/// rather than failing when a kill switch is active: the write itself
/// succeeded (a later unset of the kill switch takes effect immediately,
/// with no need to run `set` again), only the *effective* level is clamped.
///
/// `app_name` is the application whose kill switches to check. It is a
/// parameter rather than a constant because the first switch
/// [`detect_kill_switch`] tests is the app-prefixed
/// `<APP>_TELEMETRY_DISABLED`; an empty name silently narrows the check to
/// the two app-agnostic variables and reports plain success on an install
/// where telemetry is in fact off. `build_set_command` passes the same name
/// it opened the store with, so `set` and `status` answer the same question.
pub fn set_level(
    store: &TelemetryStore,
    app_name: &str,
    level: TelemetryLevel,
) -> Result<SetOutcome, TelemetryCommandError> {
    store
        .mutate(|s| s.level = Some(level))
        .map_err(map_store_error)?;

    match detect_kill_switch(app_name, &|k| std::env::var(k).ok()) {
        Some(switch) => Ok(SetOutcome::AppliedButClamped {
            effective: TelemetryLevel::Off,
            reason: format!(
                "the {} environment variable is set; telemetry stays off regardless of the \
                 configured level",
                switch.env_var(app_name)
            ),
        }),
        None => Ok(SetOutcome::Applied),
    }
}

/// Disable one probe id. Disabling a parent implicitly disables its
/// descendants (see [`super::probe::effective`]'s ancestor-prefix check) —
/// this only ever writes the one id the caller named, never the whole
/// subtree.
pub fn disable_probe(
    store: &TelemetryStore,
    registry: &ProbeRegistry,
    id: &str,
) -> Result<(), TelemetryCommandError> {
    if !registry.contains(id) {
        return Err(TelemetryCommandError::UnknownProbe(id.to_string()));
    }
    store
        .mutate(|s| {
            s.probes.insert(id.to_string(), false);
        })
        .map_err(map_store_error)?;
    Ok(())
}

/// Re-enable one probe id. Implemented as removing the id from the stored
/// map (not writing `true`): "absent means enabled" is the schema's own
/// convention (see `TelemetrySettings::probes`'s doc comment), so this keeps
/// the file from accumulating an ever-growing set of `true` entries that
/// mean nothing beyond the default. Note this cannot override an ancestor
/// that is still disabled — the probe becomes effective again only once
/// every ancestor is also enabled, which is the documented semantics of
/// [`super::probe::effective`], not a limitation of this function.
pub fn enable_probe(
    store: &TelemetryStore,
    registry: &ProbeRegistry,
    id: &str,
) -> Result<(), TelemetryCommandError> {
    if !registry.contains(id) {
        return Err(TelemetryCommandError::UnknownProbe(id.to_string()));
    }
    store
        .mutate(|s| {
            s.probes.remove(id);
        })
        .map_err(map_store_error)?;
    Ok(())
}

/// Forget every stored choice, keeping the install id. Delegates directly to
/// [`TelemetryStore::reset`], which already implements exactly this
/// contract — this wrapper exists only so the command layer has one
/// consistent `TelemetryCommandError` to match on across all five verbs.
pub fn reset(store: &TelemetryStore) -> Result<(), TelemetryCommandError> {
    store.reset().map_err(map_store_error)
}

// ── CLI-facing layer (integration-tested through a real App) ───────────────

/// Report `message` under `code` as a validation diagnostic and hand back
/// the matching `anyhow::Error` for the command to return. Every telemetry
/// leaf's user-visible failure goes through this one seam, so `stderr`
/// always carries the actual cause — an `execute` closure that merely
/// returns `Err(anyhow!(..))` without reporting first leaves `stderr` empty
/// (see `DiagnosticReporter`'s capture mechanism), which would silently
/// swallow exactly the messages these commands most need a person to see.
///
/// Mirrors `crate::config::commands`'s convention exactly: the returned
/// `anyhow::Error` carries only the bare `code` (not `message`) — the rich
/// text already went to `DiagnosticReporter`, and `CliTestHarness` exit-code
/// classification (`Ok` → 0, `UsageError` → 2, any other `Err` → 1) never
/// inspects the error's payload, so the propagated error exists only to
/// signal failure, not to carry a duplicate copy of the message.
fn fail(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    DiagnosticReporter::report(&Diagnostic {
        code,
        category: DiagnosticCategory::Validation,
        message: message.into(),
        suggestion: None,
        span: None,
    });
    anyhow::anyhow!(code)
}

/// Which `TEL0xx` code a given [`TelemetryCommandError`] should report under.
fn error_code(err: &TelemetryCommandError) -> &'static str {
    match err {
        TelemetryCommandError::UnknownProbe(_) => TEL002,
        TelemetryCommandError::StoreUnavailable(_) | TelemetryCommandError::Write(_) => TEL003,
    }
}

fn json_flag_arg(help: &'static str) -> ArgSpec {
    ArgSpec {
        name: "json",
        kind: ArgKind::Flag,
        long: Some("json"),
        value_type: ArgValueType::Bool,
        cardinality: Cardinality::Optional,
        default: Some(ArgValue::Bool(false)),
        help,
        ..Default::default()
    }
}

fn wants_json(args: &HashMap<String, ArgValue>) -> bool {
    matches!(args.get("json"), Some(ArgValue::Bool(true)))
}

fn render_status_text(report: &StatusReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "telemetry level: {} (source: {})",
        report.level, report.level_source
    ));
    if let Some(switch) = &report.kill_switch {
        lines.push(format!(
            "kill switch: {switch} (forces the effective level to off)"
        ));
    }
    match &report.endpoint {
        Some(endpoint) => lines.push(format!(
            "endpoint: {} (source: {})",
            endpoint,
            report.endpoint_source.as_deref().unwrap_or("unknown")
        )),
        None => lines.push("endpoint: none configured; nothing exports".to_string()),
    }
    lines.push(format!("attribution: {}", report.attribution));
    match &report.install_id {
        Some(id) => lines.push(format!("install id: {id}")),
        None => lines.push("install id: none (anonymous)".to_string()),
    }
    lines.push(report.policy.clone());
    lines.push(format!("settings file: {}", report.store));
    lines.push("probes:".to_string());
    for probe in &report.probes {
        let state = match (probe.enabled, probe.effective) {
            (false, _) => "disabled".to_string(),
            (true, true) => "effective".to_string(),
            (true, false) => "enabled, not currently effective".to_string(),
        };
        lines.push(format!(
            "  {} (min level {}): {} -- {}",
            probe.id, probe.min_level, state, probe.summary
        ));
    }
    lines.join("\n")
}

/// A catalog entry as printed by `telemetry info` — mirrors [`ProbeStatus`]
/// (`telemetry status`'s per-probe entry) but is generated straight from the
/// registry plus the resolved policy, so `info` and `status` can never
/// disagree about a probe's enabled/effective state. Previously this carried
/// only the probe's static description (id/min_level/summary/sends) on the
/// theory that the catalog is "a property of the binary, not of the current
/// configuration" — that rationale did not survive spec 025 line 492, which
/// documents `enabled` and `effective now` as two of the six fields `info`
/// must report.
///
/// Still resilient to a store that could not be opened: [`build_policy`]
/// already tolerates [`StoreState::Unavailable`] (`store_available: false`
/// plus a reason, with `resolve_policy` still returning a usable policy), so
/// this stays available precisely when it matters most — before an install
/// has configured anything.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeInfo {
    pub id: String,
    pub min_level: String,
    /// Whether this exact probe id has been disabled (directly, not by an
    /// ancestor). Absent from the store means enabled — see [`ProbeStatus`].
    pub enabled: bool,
    /// Whether the probe actually fires right now, computed exactly the way
    /// `status_report` computes [`ProbeStatus::effective`].
    pub effective: bool,
    pub summary: String,
    pub sends: String,
}

/// Build the full probe catalog from `policy`'s own registry (never a fresh
/// [`ProbeRegistry::with_builtins()`]) so `info` and `status` read the exact
/// same set of probes and the exact same enabled/effective computation as
/// [`status_report`].
pub fn info_catalog(policy: &TelemetryPolicy) -> Vec<ProbeInfo> {
    policy
        .registry
        .iter()
        .map(|probe| ProbeInfo {
            id: probe.id.to_string(),
            min_level: probe.min_level.as_str().to_string(),
            enabled: !policy.disabled_probes.contains(probe.id),
            effective: policy.effective(probe.id),
            summary: probe.summary.to_string(),
            sends: probe.sends.to_string(),
        })
        .collect()
}

fn render_info_text(catalog: &[ProbeInfo]) -> String {
    let mut lines = vec!["telemetry probe catalog:".to_string()];
    for probe in catalog {
        let state = match (probe.enabled, probe.effective) {
            (false, _) => "disabled".to_string(),
            (true, true) => "effective".to_string(),
            (true, false) => "enabled, not currently effective".to_string(),
        };
        lines.push(format!(
            "  {} (min level {}): {} -- {}\n    sends: {}",
            probe.id, probe.min_level, state, probe.summary, probe.sends
        ));
    }
    lines.join("\n")
}

fn build_status_command(app_name: &'static str, location: TelemetryStoreLocation) -> Command {
    Command {
        id: Arc::from("status"),
        spec: Arc::new(CommandSpec {
            summary: "Show the resolved telemetry level, attribution, and probe states",
            category: Some("Telemetry"),
            args: vec![json_flag_arg(
                "Print the status as JSON instead of a human-readable summary",
            )],
            exit_codes: vec![ExitCodeEntry {
                code: 0,
                description: "Status printed",
            }],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, args| {
            let location = location.clone();
            Box::pin(async move {
                let store = location.open(app_name);
                let policy = build_policy(app_name, &store);
                let report = status_report(&policy, store.state());
                if wants_json(&args) {
                    ctx.framework_println(&serde_json::to_string(&report)?);
                } else {
                    ctx.framework_println(&render_status_text(&report));
                }
                Ok(())
            })
        }),
    }
}

fn build_set_command(app_name: &'static str, location: TelemetryStoreLocation) -> Command {
    Command {
        id: Arc::from("set"),
        spec: Arc::new(CommandSpec {
            summary: "Set the telemetry level (off, usage, diagnostic, or debug)",
            category: Some("Telemetry"),
            args: vec![ArgSpec {
                name: "level",
                kind: ArgKind::Positional,
                value_type: ArgValueType::Enum(vec!["off", "usage", "diagnostic", "debug"]),
                cardinality: Cardinality::Required,
                help: "The telemetry level to apply: off, usage, diagnostic, or debug",
                ..Default::default()
            }],
            exit_codes: vec![
                ExitCodeEntry {
                    code: 0,
                    description: "Level applied (possibly clamped by a kill switch)",
                },
                ExitCodeEntry {
                    code: 1,
                    description: "The level was invalid, or the settings file could not be written",
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, args| {
            let location = location.clone();
            Box::pin(async move {
                let level_str = match args.get("level") {
                    Some(ArgValue::Enum(s)) | Some(ArgValue::Str(s)) => s.clone(),
                    _ => {
                        return Err(fail(
                            TEL001,
                            "a level is required: one of off, usage, diagnostic, debug",
                        ));
                    }
                };
                let level: TelemetryLevel = match level_str.parse() {
                    Ok(level) => level,
                    Err(_) => {
                        return Err(fail(
                            TEL001,
                            format!(
                                "{level_str:?} is not a valid telemetry level; valid levels are: \
                                 off, usage, diagnostic, debug"
                            ),
                        ));
                    }
                };

                let store = location.open(app_name);
                match set_level(&store, app_name, level) {
                    Ok(SetOutcome::Applied) => {
                        ctx.framework_println(&format!("telemetry level set to {level}"));
                        Ok(())
                    }
                    Ok(SetOutcome::AppliedButClamped { effective, reason }) => {
                        ctx.framework_println(&format!(
                            "telemetry level saved as {level}, but the effective level is \
                             {effective}: {reason}"
                        ));
                        Ok(())
                    }
                    // `set_level`'s only error path is `map_store_error`
                    // (never `UnknownProbe`), so this is always TEL003.
                    Err(e) => Err(fail(TEL003, e.to_string())),
                }
            })
        }),
    }
}

fn build_info_command(app_name: &'static str, location: TelemetryStoreLocation) -> Command {
    Command {
        id: Arc::from("info"),
        spec: Arc::new(CommandSpec {
            summary: "List every telemetry probe this build can emit, and what it sends",
            category: Some("Telemetry"),
            args: vec![json_flag_arg(
                "Print the catalog as JSON instead of a human-readable list",
            )],
            exit_codes: vec![ExitCodeEntry {
                code: 0,
                description: "Catalog printed",
            }],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, args| {
            let location = location.clone();
            Box::pin(async move {
                let store = location.open(app_name);
                let policy = build_policy(app_name, &store);
                let catalog = info_catalog(&policy);
                if wants_json(&args) {
                    ctx.framework_println(&serde_json::to_string(&catalog)?);
                } else {
                    ctx.framework_println(&render_info_text(&catalog));
                }
                Ok(())
            })
        }),
    }
}

fn build_disable_command(app_name: &'static str, location: TelemetryStoreLocation) -> Command {
    Command {
        id: Arc::from("disable"),
        spec: Arc::new(CommandSpec {
            summary: "Disable one probe id (and, implicitly, everything under it)",
            category: Some("Telemetry"),
            args: vec![ArgSpec {
                name: "probe_id",
                kind: ArgKind::Positional,
                value_type: ArgValueType::String,
                cardinality: Cardinality::Required,
                help: "The probe id to disable, e.g. cli.command.arg_values",
                ..Default::default()
            }],
            exit_codes: vec![
                ExitCodeEntry {
                    code: 0,
                    description: "Probe disabled",
                },
                ExitCodeEntry {
                    code: 1,
                    description: "Unknown probe id, or the settings file could not be written",
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, args| {
            let location = location.clone();
            Box::pin(async move {
                let Some(ArgValue::Str(id)) = args.get("probe_id") else {
                    return Err(fail(TEL001, "a probe id is required"));
                };
                let store = location.open(app_name);
                match disable_probe(&store, &ProbeRegistry::with_builtins(), id) {
                    Ok(()) => {
                        ctx.framework_println(&format!("probe {id} disabled"));
                        Ok(())
                    }
                    Err(e) => Err(fail(error_code(&e), e.to_string())),
                }
            })
        }),
    }
}

fn build_enable_command(app_name: &'static str, location: TelemetryStoreLocation) -> Command {
    Command {
        id: Arc::from("enable"),
        spec: Arc::new(CommandSpec {
            summary: "Re-enable one probe id",
            category: Some("Telemetry"),
            args: vec![ArgSpec {
                name: "probe_id",
                kind: ArgKind::Positional,
                value_type: ArgValueType::String,
                cardinality: Cardinality::Required,
                help: "The probe id to re-enable, e.g. cli.command.arg_values",
                ..Default::default()
            }],
            exit_codes: vec![
                ExitCodeEntry {
                    code: 0,
                    description: "Probe enabled",
                },
                ExitCodeEntry {
                    code: 1,
                    description: "Unknown probe id, or the settings file could not be written",
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, args| {
            let location = location.clone();
            Box::pin(async move {
                let Some(ArgValue::Str(id)) = args.get("probe_id") else {
                    return Err(fail(TEL001, "a probe id is required"));
                };
                let store = location.open(app_name);
                match enable_probe(&store, &ProbeRegistry::with_builtins(), id) {
                    Ok(()) => {
                        ctx.framework_println(&format!("probe {id} enabled"));
                        Ok(())
                    }
                    Err(e) => Err(fail(error_code(&e), e.to_string())),
                }
            })
        }),
    }
}

fn build_reset_command(app_name: &'static str, location: TelemetryStoreLocation) -> Command {
    Command {
        id: Arc::from("reset"),
        spec: Arc::new(CommandSpec {
            summary: "Forget every stored telemetry choice (the install id is kept)",
            category: Some("Telemetry"),
            exit_codes: vec![
                ExitCodeEntry {
                    code: 0,
                    description: "Settings reset",
                },
                ExitCodeEntry {
                    code: 1,
                    description: "The settings file could not be written",
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: Some(vec!["app".to_string()]),
        meta: None,
        execute: Arc::new(move |ctx, _args| {
            let location = location.clone();
            Box::pin(async move {
                let store = location.open(app_name);
                match reset(&store) {
                    Ok(()) => {
                        ctx.framework_println(
                            "telemetry settings reset; nothing has been chosen yet",
                        );
                        Ok(())
                    }
                    // `reset` only ever calls `TelemetryStore::reset`, which
                    // goes through `map_store_error` — never `UnknownProbe`.
                    Err(e) => Err(fail(TEL003, e.to_string())),
                }
            })
        }),
    }
}

/// Register the `telemetry` group and its six leaf commands. Called from
/// `AppBuilder::build` only when `self.deployment.is_end_user()` (see the
/// guard there) — a `Service` deployment never sees this group registered
/// at all.
pub(crate) fn register_telemetry_commands(
    registry: &mut CommandRegistry,
    app_name: &'static str,
    location: TelemetryStoreLocation,
) -> anyhow::Result<()> {
    let group_path = CommandPath::root_for("telemetry");
    registry
        .register_group(
            &group_path,
            GroupMetadata {
                summary: "Inspect and control this application's telemetry",
                hidden: false,
            },
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    registry
        .register_at(
            &CommandPath::new(&["telemetry", "status"]).unwrap(),
            build_status_command(app_name, location.clone()),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(
            &CommandPath::new(&["telemetry", "set"]).unwrap(),
            build_set_command(app_name, location.clone()),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(
            &CommandPath::new(&["telemetry", "info"]).unwrap(),
            build_info_command(app_name, location.clone()),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(
            &CommandPath::new(&["telemetry", "disable"]).unwrap(),
            build_disable_command(app_name, location.clone()),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(
            &CommandPath::new(&["telemetry", "enable"]).unwrap(),
            build_enable_command(app_name, location.clone()),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(
            &CommandPath::new(&["telemetry", "reset"]).unwrap(),
            build_reset_command(app_name, location),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}
