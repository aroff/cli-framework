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

/// The framework's own variables, for the application's root `--help`.
///
/// Derived from the published manifest rather than from a hand-written list
/// so that what help advertises is exactly what [`scan_environment`] honours:
/// the three kill switches, one variable per settable `telemetry.*` leaf, and
/// one pattern row for the probe switches. The probe switches are collapsed
/// to `<APP>_TELEMETRY_<PROBE>_ENABLED` on purpose — twenty built-in probes
/// would otherwise put twenty lines into every application's help, and the
/// same pattern covers an app's operational probes. The manifest's
/// `local_only` leaves (`install_id`, `notice_shown`) are state the
/// framework writes, not settings a person sets, and are not listed. The
/// `OTEL_*` rows are the variables the build-time resolution and the
/// exporter read on the current API; `OTEL_EXPORTER_OTLP_PROTOCOL` is read
/// only by the deprecated `TelemetryConfig::from_env` and is not listed.
pub(crate) fn help_entries(app: &str, manifest: &ConfigManifest) -> Vec<(String, String)> {
    let prefix = env_var_prefix(app);
    let off = "Set to 1 to turn telemetry off entirely.";
    let mut out: Vec<(String, String)> = vec![
        (format!("{prefix}_TELEMETRY_DISABLED"), off.to_string()),
        ("DO_NOT_TRACK".to_string(), off.to_string()),
        (
            "OTEL_SDK_DISABLED".to_string(),
            "Set to true to turn telemetry off entirely.".to_string(),
        ),
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT".to_string(),
            "OTLP collector URL; overrides the telemetry endpoint setting.".to_string(),
        ),
        (
            "OTEL_EXPORTER_OTLP_HEADERS".to_string(),
            "Headers sent with every export, as comma-separated key=value pairs.".to_string(),
        ),
        (
            "OTEL_TRACES_SAMPLER_ARG".to_string(),
            "Fraction of traces exported, 0.0 to 1.0.".to_string(),
        ),
        (
            "OTEL_SERVICE_NAME".to_string(),
            "Service name reported to the collector; defaults to the application name.".to_string(),
        ),
        (
            "OTEL_RESOURCE_ATTRIBUTES".to_string(),
            "Extra resource attributes, as comma-separated key=value pairs.".to_string(),
        ),
    ];

    let mut has_probe_switch = false;
    for leaf in manifest.iter_leaves() {
        let Some(rest) = leaf.path.strip_prefix("telemetry.") else {
            continue;
        };
        if leaf.field.local_only {
            continue;
        }
        if rest.contains('.') {
            // Nested under a probe id: `telemetry.<probe>.enabled`.
            has_probe_switch = true;
            continue;
        }
        let description = leaf
            .field
            .description
            .clone()
            .or_else(|| leaf.field.label.clone())
            .unwrap_or_else(|| leaf.path.clone());
        out.push((env_var_name(app, &leaf.path), description));
    }
    if has_probe_switch {
        out.push((
            format!("{prefix}_TELEMETRY_<PROBE>_ENABLED"),
            "Set to 0 to disable one telemetry probe; <PROBE> is its id upper-cased with dots \
             as underscores."
                .to_string(),
        ));
    }
    out
}

#[cfg(test)]
mod help_entry_tests {
    use super::*;
    use crate::telemetry::manifest::{merge_telemetry_section, telemetry_only_manifest};
    use crate::telemetry::probe::ProbeRegistry;

    fn names(entries: &[(String, String)]) -> Vec<&str> {
        entries.iter().map(|(n, _)| n.as_str()).collect()
    }

    #[test]
    fn lists_kill_switches_settings_and_one_probe_pattern() {
        let manifest = telemetry_only_manifest("my-app", &ProbeRegistry::with_builtins(), None);
        let entries = help_entries("my-app", &manifest);
        let names = names(&entries);

        for expected in [
            "MY_APP_TELEMETRY_DISABLED",
            "DO_NOT_TRACK",
            "OTEL_SDK_DISABLED",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_EXPORTER_OTLP_HEADERS",
            "OTEL_TRACES_SAMPLER_ARG",
            "MY_APP_TELEMETRY_LEVEL",
            "MY_APP_TELEMETRY_ATTRIBUTION",
            "MY_APP_TELEMETRY_ENDPOINT",
            "MY_APP_TELEMETRY_<PROBE>_ENABLED",
        ] {
            assert!(names.contains(&expected), "missing {expected} in {names:?}");
        }

        let level = entries
            .iter()
            .find(|(n, _)| n == "MY_APP_TELEMETRY_LEVEL")
            .map(|(_, d)| d.as_str())
            .unwrap();
        assert!(
            level.starts_with("How much this application reports"),
            "the level row must reuse the manifest's description, got {level:?}"
        );

        let enabled_rows: Vec<_> = names.iter().filter(|n| n.ends_with("_ENABLED")).collect();
        assert_eq!(
            enabled_rows,
            vec![&"MY_APP_TELEMETRY_<PROBE>_ENABLED"],
            "probe switches collapse to one pattern row, never one row per probe"
        );
    }

    #[test]
    fn omits_local_only_state_leaves() {
        let manifest = telemetry_only_manifest("demo", &ProbeRegistry::with_builtins(), None);
        let entries = help_entries("demo", &manifest);
        let names = names(&entries);
        assert!(!names.contains(&"DEMO_TELEMETRY_INSTALL_ID"), "{names:?}");
        assert!(!names.contains(&"DEMO_TELEMETRY_NOTICE_SHOWN"), "{names:?}");
    }

    #[test]
    fn omits_the_probe_pattern_when_no_probe_is_registered() {
        let manifest = telemetry_only_manifest("demo", &ProbeRegistry::new(), None);
        let entries = help_entries("demo", &manifest);
        let names = names(&entries);
        assert!(
            !names.iter().any(|n| n.ends_with("_ENABLED")),
            "no probes, no switch row: {names:?}"
        );
        assert!(names.contains(&"DEMO_TELEMETRY_LEVEL"));
    }

    #[test]
    fn ignores_the_applications_own_manifest_fields() {
        use crate::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
        let app_field = FieldManifest {
            key: "api_url".to_string(),
            kind: FieldKind::Url,
            default: None,
            label: None,
            description: Some("The app's API".to_string()),
            group: None,
            scope: Scope::Machine,
            platforms: Vec::new(),
            secret: false,
            local_only: false,
            protected: false,
            manageable: true,
            enforceable: true,
            restart_required: false,
            constraints: None,
        };
        let app_manifest = ConfigManifest::new("demo", vec![app_field]);
        let manifest =
            merge_telemetry_section(app_manifest, &ProbeRegistry::with_builtins(), None).unwrap();
        let entries = help_entries("demo", &manifest);
        let names = names(&entries);
        assert!(!names.contains(&"DEMO_API_URL"), "{names:?}");
        assert!(names.contains(&"DEMO_TELEMETRY_LEVEL"));
    }
}
