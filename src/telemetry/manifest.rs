// src/telemetry/manifest.rs
//! The `telemetry` section of the application's published config manifest.
//!
//! There is exactly one manifest per application. Telemetry does not publish
//! a second one — it contributes a section to the app's, so an administrator
//! sees one document and the resolver has one keyspace.
//!
//! The policy flags carry the spec's rules. `telemetry.level` is
//! `manageable: true, enforceable: false`: an organisation may *recommend* a
//! telemetry level and may not *mandate* one. That single pair of booleans is
//! what makes the config service reject an enforced level
//! (`validate_stored_policy`) and the client drop one that arrives anyway
//! (`WarningReason::NotEnforceableInEnforced`) — the rule lives in the
//! manifest, not in two hand-written checks that could drift.

use super::axes::{Attribution, TelemetryLevel};
use super::probe::ProbeRegistry;
use crate::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
use serde_json::Value;

/// The one top-level key the framework claims in an app's manifest.
pub const TELEMETRY_SECTION_KEY: &str = "telemetry";

/// Refused at build time rather than papered over at runtime.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestMergeError {
    #[error(
        "TM001: this application's config manifest already declares a top-level 'telemetry' \
         key; the framework owns that key. Rename the application's field."
    )]
    AppOwnsTelemetryKey,
}

fn field(key: &str, kind: FieldKind) -> FieldManifest {
    FieldManifest {
        key: key.to_string(),
        kind,
        default: None,
        label: None,
        description: None,
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
    }
}

fn level_field() -> FieldManifest {
    FieldManifest {
        default: Some(Value::String(TelemetryLevel::Off.as_str().to_string())),
        label: Some("Telemetry level".to_string()),
        description: Some(
            "How much this application reports about its own operation. 'off' sends nothing."
                .to_string(),
        ),
        // Recommendable, never enforceable: an organisation may suggest a
        // telemetry level; consent is not something it can mandate.
        enforceable: false,
        restart_required: true,
        ..field(
            "level",
            FieldKind::Enum {
                values: TelemetryLevel::ALL
                    .iter()
                    .map(|l| l.as_str().to_string())
                    .collect(),
            },
        )
    }
}

fn attribution_field() -> FieldManifest {
    FieldManifest {
        default: Some(Value::String(
            Attribution::Pseudonymous.as_str().to_string(),
        )),
        label: Some("Attribution".to_string()),
        description: Some(
            "Whether reports carry no identifier, a per-install identifier, or an account."
                .to_string(),
        ),
        enforceable: false,
        restart_required: true,
        ..field(
            "attribution",
            FieldKind::Enum {
                values: Attribution::ALL
                    .iter()
                    .map(|a| a.as_str().to_string())
                    .collect(),
            },
        )
    }
}

fn endpoint_field(default_endpoint: Option<&str>) -> FieldManifest {
    FieldManifest {
        default: Some(Value::String(
            default_endpoint.unwrap_or_default().to_string(),
        )),
        label: Some("Telemetry endpoint".to_string()),
        description: Some(
            "The OTLP collector reports are sent to. Empty means nothing is sent.".to_string(),
        ),
        restart_required: true,
        ..field("endpoint", FieldKind::Url)
    }
}

fn install_id_field() -> FieldManifest {
    FieldManifest {
        label: Some("Install identifier".to_string()),
        description: Some(
            "A random identifier for this installation. Never leaves this machine except \
             inside this application's own reports."
                .to_string(),
        ),
        local_only: true,
        protected: true,
        manageable: false,
        enforceable: false,
        ..field("install_id", FieldKind::Str)
    }
}

fn notice_shown_field() -> FieldManifest {
    FieldManifest {
        label: Some("Notice shown".to_string()),
        description: Some("The telemetry level this installation was last told about.".to_string()),
        local_only: true,
        protected: true,
        manageable: false,
        enforceable: false,
        ..field("notice_shown", FieldKind::Str)
    }
}

/// One `<probe>.enabled` switch per registered probe, nested so that the
/// dotted probe id becomes a path of sections: `cli.command.args` becomes
/// `telemetry.cli.command.args.enabled`.
fn probe_switches(registry: &ProbeRegistry) -> Vec<FieldManifest> {
    let mut roots: Vec<FieldManifest> = Vec::new();
    for spec in registry.iter() {
        let mut cursor = &mut roots;
        for segment in spec.id.split('.') {
            let position = cursor.iter().position(|f| f.key == segment);
            let index = match position {
                Some(i) => i,
                None => {
                    cursor.push(field(segment, FieldKind::Section { fields: Vec::new() }));
                    cursor.len() - 1
                }
            };
            match &mut cursor[index].kind {
                FieldKind::Section { fields } => cursor = fields,
                _ => unreachable!("probe path segments are always sections"),
            }
        }
        cursor.push(FieldManifest {
            default: Some(Value::Bool(true)),
            label: Some(format!("{} probe", spec.id)),
            description: Some(format!("{} Sends: {}", spec.summary, spec.sends)),
            restart_required: true,
            ..field("enabled", FieldKind::Bool)
        });
    }
    roots
}

/// The `telemetry` section itself.
pub fn telemetry_section(
    registry: &ProbeRegistry,
    default_endpoint: Option<&str>,
) -> FieldManifest {
    let mut fields = vec![
        level_field(),
        attribution_field(),
        endpoint_field(default_endpoint),
        install_id_field(),
        notice_shown_field(),
    ];
    fields.extend(probe_switches(registry));
    field(TELEMETRY_SECTION_KEY, FieldKind::Section { fields })
}

/// Add the section to an application's own manifest.
pub fn merge_telemetry_section(
    app: ConfigManifest,
    registry: &ProbeRegistry,
    default_endpoint: Option<&str>,
) -> Result<ConfigManifest, ManifestMergeError> {
    if app.fields.iter().any(|f| f.key == TELEMETRY_SECTION_KEY) {
        return Err(ManifestMergeError::AppOwnsTelemetryKey);
    }
    let mut fields = app.fields;
    fields.push(telemetry_section(registry, default_endpoint));
    Ok(ConfigManifest::new(app.app, fields))
}

/// The manifest for an application that publishes none of its own. The
/// telemetry tree is still resolvable and still administrable; the app's
/// `config` commands are simply not auto-registered.
pub fn telemetry_only_manifest(
    app: &str,
    registry: &ProbeRegistry,
    default_endpoint: Option<&str>,
) -> ConfigManifest {
    ConfigManifest::new(app, vec![telemetry_section(registry, default_endpoint)])
}
