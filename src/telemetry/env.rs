// src/telemetry/env.rs
//! The environment layer for the `telemetry.` subtree.
//!
//! Variable names are computed **from the manifest**, one per leaf, and then
//! looked up. The reverse direction — reading `DEMO_TELEMETRY_A_B` and
//! deducing a dotted path — is ambiguous, because `a_b` and `a.b` produce the
//! same variable. Computing forwards makes the mapping total and testable.

use super::policy::env_var_prefix;
use crate::config::manifest::{ConfigManifest, FieldKind};
use serde_json::{Map, Value};

/// The environment variable that sets `path` for `app`.
pub fn env_var_name(app: &str, path: &str) -> String {
    format!(
        "{}_{}",
        env_var_prefix(app),
        path.to_ascii_uppercase().replace(['.', '-'], "_")
    )
}

/// What the environment had to say about the telemetry tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvScan {
    /// Values keyed by dotted manifest path, ready for
    /// `ResolutionInput::environment`.
    pub values: Map<String, Value>,
    /// `<APP>_TELEMETRY_*` variables that matched no field. Almost always a
    /// typo, and silence would make it invisible, so these become a startup
    /// warning and the `telemetry.env` doctor finding.
    pub unmatched: Vec<String>,
}

fn typed(kind: &FieldKind, raw: &str) -> Value {
    match kind {
        FieldKind::Bool => match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Value::Bool(true),
            "0" | "false" | "no" | "off" => Value::Bool(false),
            _ => Value::String(raw.to_string()),
        },
        FieldKind::Int | FieldKind::Duration => raw
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .unwrap_or_else(|_| Value::String(raw.to_string())),
        FieldKind::Float => serde_json::Number::from_f64(raw.parse::<f64>().unwrap_or(f64::NAN))
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(raw.to_string())),
        _ => Value::String(raw.to_string()),
    }
}

/// Read the telemetry subtree out of `vars`.
///
/// `vars` is passed in rather than read from the process so this stays pure:
/// `std::env` is global mutable state shared by every test in a binary.
pub fn scan_environment(
    app: &str,
    manifest: &ConfigManifest,
    vars: &[(String, String)],
) -> EnvScan {
    let prefix = env_var_prefix(app);
    let telemetry_prefix = format!("{prefix}_TELEMETRY_");
    let kill_switch = format!("{prefix}_TELEMETRY_DISABLED");

    let mut known: Vec<(String, String, FieldKind)> = Vec::new();
    for leaf in manifest.iter_leaves() {
        if leaf.path == "telemetry" || leaf.path.starts_with("telemetry.") {
            known.push((
                env_var_name(app, &leaf.path),
                leaf.path.clone(),
                leaf.field.kind.clone(),
            ));
        }
    }

    let mut scan = EnvScan::default();
    for (name, raw) in vars {
        if let Some((_, path, kind)) = known.iter().find(|(var, _, _)| var == name) {
            scan.values.insert(path.clone(), typed(kind, raw));
        } else if name.starts_with(&telemetry_prefix) && name != &kill_switch {
            scan.unmatched.push(name.clone());
        }
    }
    scan.unmatched.sort();
    scan
}
