//! Built-in `config` command group: `config show`, `config manifest`,
//! `config profile`, `config refresh` (spec 021, "Command surface" — the one
//! piece of that PRD not implemented alongside the manifest/resolver/managed
//! client).
//!
//! Follows the exact `Command`/`CommandSpec`/`CommandPath`/`GroupMetadata`
//! construction style of [`crate::auth::commands`] (`auth login`/`logout`/
//! `status`/`token`) — no `#[derive(CommandSpec)]`, matching how that group's
//! mostly-argument-free commands are declared by hand. These are read-only
//! diagnostic commands, exactly like `auth status`/`auth token`: no
//! `CommandGate` or consent is involved.
//!
//! This whole module lives behind `config-managed`, even though `config
//! show`/`config manifest` only ever call
//! [`AppContext::opt_config_manifest`]/[`AppContext::opt_config_handle`] —
//! neither of which is specific to `config-managed` — because the
//! auto-registration call site in `AppBuilder::build` only exists under that
//! feature (see the comment there): `config profile`/`config refresh` are
//! the pieces that genuinely need a
//! [`PolicyClient`][crate::config::managed::PolicyClient], and spec 021
//! treats the four commands as one group rather than four independently
//! opt-in pieces. `config show`/`config profile` still degrade gracefully
//! (reporting "unmanaged") when [`AppContext::opt_policy_client`] returns
//! `None` — an app that called `with_config_manifest` but never
//! `with_policy_client` gets a working `config show`/`config manifest` and an
//! honest "unmanaged" from `config profile`/`config refresh`.

use crate::app::context::AppContext;
use crate::app::diagnostic_reporter::DiagnosticReporter;
use crate::cli_output::{format_json, format_table, ColumnDef, GridData};
use crate::command::{Command, CommandRegistry};
use crate::config::managed::PolicyOutcome;
use crate::config::resolution::{flatten_to_paths, resolve, Layer, ResolutionInput, ResolvedEntry};
use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::{CFG001, CFG002, CFG003, CFG004};
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandPath, CommandSpec, ExitCodeEntry, GroupMetadata};
use crate::spec::value::ArgValue;
use std::collections::HashMap;
use std::sync::Arc;

/// Register the `config` group and its four leaf commands.
pub(crate) fn register_config_commands(
    registry: &mut CommandRegistry,
    _app_name: &'static str,
) -> anyhow::Result<()> {
    let group_path = CommandPath::root_for("config");
    registry
        .register_group(
            &group_path,
            GroupMetadata {
                summary: "Inspect resolved configuration, policy, and the app's manifest",
                hidden: false,
            },
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let show_path = CommandPath::new(&["config", "show"]).unwrap();
    let manifest_path = CommandPath::new(&["config", "manifest"]).unwrap();
    let profile_path = CommandPath::new(&["config", "profile"]).unwrap();
    let refresh_path = CommandPath::new(&["config", "refresh"]).unwrap();

    registry
        .register_at(&show_path, build_show_command())
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(&manifest_path, build_manifest_command())
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(&profile_path, build_profile_command())
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    registry
        .register_at(&refresh_path, build_refresh_command())
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}

// ── shared helpers ───────────────────────────────────────────────────────────

/// The `--format` `ArgSpec`, shared by every subcommand that has one — same
/// `ArgKind::Option` + `ArgValueType::Enum` idiom as the built-in `spec`
/// command's `--format json|yaml|markdown` (`src/command_surface/command.rs`).
fn format_arg(variants: Vec<&'static str>, default: &'static str, help: &'static str) -> ArgSpec {
    ArgSpec {
        name: "format",
        kind: ArgKind::Option,
        value_type: ArgValueType::Enum(variants),
        cardinality: Cardinality::Optional,
        default: Some(ArgValue::Enum(default.to_string())),
        help,
        ..Default::default()
    }
}

/// Read the parsed `--format` value, falling back to `default` for anything
/// unexpected (R4a already rejects an invalid Enum token at parse time; this
/// mirrors the defensive fallback the `spec` command uses for its own
/// `format` arg).
fn format_value(args: &HashMap<String, ArgValue>, default: &str) -> String {
    match args.get("format") {
        Some(ArgValue::Enum(s)) | Some(ArgValue::Str(s)) => s.clone(),
        _ => default.to_string(),
    }
}

fn report_no_manifest() {
    DiagnosticReporter::report(&Diagnostic {
        code: CFG001,
        category: DiagnosticCategory::Validation,
        message: "no config manifest registered; call `AppBuilder::with_config_manifest` \
                  to enable the `config` command group"
            .to_string(),
        suggestion: None,
        span: None,
    });
}

/// A [`Layer`]'s wire label — reuses `Layer`'s own `snake_case` `Serialize`
/// impl (already covered by `layer_serializes_snake_case` in
/// `config::resolution::provenance`) rather than re-deriving the mapping.
fn layer_label(layer: Layer) -> String {
    serde_json::to_value(layer)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Render a JSON value for table display: an unquoted string for
/// `Value::String`, compact JSON for everything else.
fn display_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

// ── config show ──────────────────────────────────────────────────────────────

fn build_show_command() -> Command {
    Command {
        id: Arc::from("show"),
        spec: Arc::new(CommandSpec {
            summary: "Show resolved configuration values with per-field provenance",
            category: Some("Configuration"),
            args: vec![format_arg(
                vec!["table", "json"],
                "table",
                "Output format: table or json (default: table)",
            )],
            exit_codes: vec![
                ExitCodeEntry {
                    code: 0,
                    description: "Resolved values printed",
                },
                ExitCodeEntry {
                    code: 1,
                    description: "No config manifest registered",
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
            // Borrowed accessors: extract owned copies before entering the
            // async block, matching the `spec` command's "access the
            // registry synchronously" precedent — avoids holding a borrow of
            // `ctx` across an await point.
            let manifest = ctx.opt_config_manifest().cloned();
            let config_file = ctx
                .opt_config_handle()
                .and_then(|h| h.current_json().ok())
                .map(|v| flatten_to_paths(&v))
                .unwrap_or_default();

            Box::pin(async move {
                let Some(manifest) = manifest else {
                    report_no_manifest();
                    return Err(anyhow::anyhow!("CFG001"));
                };

                // Owned-`Arc` accessor: called inside the async block,
                // matching `auth`'s `opt_token_provider()` idiom.
                //
                // Distinguish "never fetched yet" (`Ok(None)`) from "the
                // cache is corrupt/unreadable" (`Err(_)`) — collapsing both
                // into `None` via `.ok().flatten()` would silently present a
                // real cache-read error to the user as "this machine isn't
                // managed," which is actively misleading for an operator
                // debugging why an enforced setting isn't applying. `config
                // show` still proceeds on local/default values either way
                // (that matches spec intent for a genuinely unmanaged app),
                // but a cache error gets a visible warning first.
                let policy = match ctx.opt_policy_client() {
                    Some(pc) => match pc.cached_policy() {
                        Ok(policy) => policy,
                        Err(e) => {
                            DiagnosticReporter::write_plain(&format!(
                                "warning: policy cache unreadable, showing local values \
                                 only: {e}\n"
                            ));
                            None
                        }
                    },
                    None => None,
                };

                let input = ResolutionInput {
                    recommended: policy
                        .as_ref()
                        .map(|p| p.recommended.clone())
                        .unwrap_or_default(),
                    enforced: policy
                        .as_ref()
                        .map(|p| p.enforced.clone())
                        .unwrap_or_default(),
                    config_file,
                    ..Default::default()
                };
                let resolved = resolve(&manifest, &input);
                let mut entries = resolved.entries();
                entries.sort_by(|a, b| a.path.cmp(&b.path));

                let format = format_value(&args, "table");
                let rendered = if format == "json" {
                    render_show_json(&entries)?
                } else {
                    render_show_table(&entries)?
                };
                ctx.framework_println(&rendered);
                Ok(())
            })
        }),
    }
}

#[derive(serde::Serialize)]
struct ShowRow {
    field: String,
    value: String,
    layer: String,
    locked: bool,
}

fn render_show_table(entries: &[ResolvedEntry]) -> anyhow::Result<String> {
    let rows: Vec<ShowRow> = entries
        .iter()
        .map(|e| ShowRow {
            field: e.path.clone(),
            value: display_value(&e.value),
            layer: layer_label(e.provenance.layer),
            locked: e.provenance.locked,
        })
        .collect();
    let grid = GridData {
        rows,
        columns: vec![
            ColumnDef {
                name: "Field".to_string(),
                width_hint: None,
                alignment: None,
            },
            ColumnDef {
                name: "Value".to_string(),
                width_hint: None,
                alignment: None,
            },
            ColumnDef {
                name: "Layer".to_string(),
                width_hint: None,
                alignment: None,
            },
            ColumnDef {
                name: "Locked".to_string(),
                width_hint: None,
                alignment: None,
            },
        ],
        row_headers: None,
    };
    format_table(&grid)
}

fn render_show_json(entries: &[ResolvedEntry]) -> anyhow::Result<String> {
    let value: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "field": e.path,
                "value": e.value,
                "layer": layer_label(e.provenance.layer),
                "locked": e.provenance.locked,
            })
        })
        .collect();
    format_json(&value)
}

// ── config manifest ──────────────────────────────────────────────────────────

fn build_manifest_command() -> Command {
    Command {
        id: Arc::from("manifest"),
        spec: Arc::new(CommandSpec {
            summary: "Export the registered config manifest as pretty-printed JSON",
            category: Some("Configuration"),
            exit_codes: vec![
                ExitCodeEntry {
                    code: 0,
                    description: "Manifest printed",
                },
                ExitCodeEntry {
                    code: 1,
                    description: "No config manifest registered",
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
            let manifest = ctx.opt_config_manifest().cloned();
            Box::pin(async move {
                let Some(manifest) = manifest else {
                    report_no_manifest();
                    return Err(anyhow::anyhow!("CFG001"));
                };
                let rendered = format_json(&manifest)?;
                ctx.framework_println(&rendered);
                Ok(())
            })
        }),
    }
}

// ── config profile ───────────────────────────────────────────────────────────

fn build_profile_command() -> Command {
    Command {
        id: Arc::from("profile"),
        spec: Arc::new(CommandSpec {
            summary: "Show the active org profile and policy version",
            category: Some("Configuration"),
            args: vec![format_arg(
                vec!["text", "json"],
                "text",
                "Output format: text or json (default: text)",
            )],
            exit_codes: vec![
                ExitCodeEntry {
                    code: 0,
                    description: "Profile status printed (managed or unmanaged)",
                },
                ExitCodeEntry {
                    code: 1,
                    description: "Cached policy could not be read",
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
            Box::pin(async move {
                let format = format_value(&args, "text");
                // No client at all and "client exists but nothing cached
                // yet" are both, honestly, "unmanaged" from this command's
                // point of view (spec 021: "an application that is simply
                // not managed" must report cleanly, not error).
                let cached = match ctx.opt_policy_client() {
                    Some(pc) => pc.cached_policy(),
                    None => Ok(None),
                };
                match cached {
                    Ok(Some(policy)) => {
                        if format == "json" {
                            let payload = serde_json::json!({
                                "managed": true,
                                "profile": policy.profile,
                                "policy_version": policy.policy_version,
                            });
                            ctx.framework_println(&serde_json::to_string(&payload)?);
                        } else {
                            ctx.framework_println(&format!(
                                "Profile: {} (policy version {})",
                                policy.profile, policy.policy_version
                            ));
                        }
                        Ok(())
                    }
                    Ok(None) => {
                        if format == "json" {
                            ctx.framework_println(
                                r#"{"managed":false,"profile":null,"policy_version":null}"#,
                            );
                        } else {
                            ctx.framework_println(
                                "unmanaged (no policy has ever been fetched for this app)",
                            );
                        }
                        Ok(())
                    }
                    Err(e) => {
                        // `CFG004`, not `CFG003`: this is a corrupt/unreadable
                        // cache, not a request-time denial or refresh
                        // failure — a genuinely different, distinctly
                        // actionable condition (see `CFG004`'s docs).
                        DiagnosticReporter::report(&Diagnostic {
                            code: CFG004,
                            category: DiagnosticCategory::Validation,
                            message: format!("cached policy could not be read: {e}"),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("CFG004"))
                    }
                }
            })
        }),
    }
}

// ── config refresh ───────────────────────────────────────────────────────────

fn build_refresh_command() -> Command {
    Command {
        id: Arc::from("refresh"),
        spec: Arc::new(CommandSpec {
            summary: "Force a policy refetch, bypassing cache freshness",
            category: Some("Configuration"),
            args: vec![format_arg(
                vec!["text", "json"],
                "text",
                "Output format: text or json (default: text)",
            )],
            exit_codes: vec![
                ExitCodeEntry {
                    code: 0,
                    description: "Refresh completed: fresh, cache fallback, or unmanaged",
                },
                ExitCodeEntry {
                    code: 1,
                    description:
                        "No PolicyClient registered, access denied, or an unrecoverable refresh failure",
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
            // Owned, extracted up front: `refresh_managed_config`'s
            // type-erased counterpart (`apply_policy_outcome_to_handle`)
            // needs `&dyn ConfigHandle` *after* the fetch below, not before —
            // borrowing `ctx` for it now (across the `.await`) would fight
            // the borrow checker for no reason, since a fresh call to
            // `ctx.opt_config_handle()` after the fetch is just as valid and
            // costs nothing extra. Only `manifest` (owned via `.cloned()`) is
            // worth extracting before the fetch.
            let manifest = ctx.opt_config_manifest().cloned();

            Box::pin(async move {
                let format = format_value(&args, "text");
                let Some(pc) = ctx.opt_policy_client() else {
                    DiagnosticReporter::report(&Diagnostic {
                        code: CFG002,
                        category: DiagnosticCategory::Validation,
                        message: "no PolicyClient registered; call \
                                  `AppBuilder::with_policy_client` to enable managed \
                                  configuration"
                            .to_string(),
                        suggestion: None,
                        span: None,
                    });
                    return Err(anyhow::anyhow!("CFG002"));
                };

                // `fetch()` applies spec 021's failure-mapping table exactly
                // as written (see `PolicyClient`'s own module docs) — this
                // command surfaces whichever outcome it returns; it never
                // second-guesses or loosens that mapping (a 401-after-retry
                // or 403 stays `Denied` and never reads the cache here
                // either).
                let fetch_result = pc.fetch().await;

                // With a manifest AND a registered `ConfigStore` (reachable
                // through `opt_config_handle`, exactly like `config show`),
                // fold a successful outcome into the *running* store via
                // `set_current_json_and_notify` — this is what makes `config
                // refresh` actually refresh the running application's config,
                // not just report what the server said. Without either, this
                // command's behavior is unchanged from before this fold
                // existed: it only ever reports the outcome.
                let result = match fetch_result {
                    Ok(outcome) => {
                        if let (Some(manifest), Some(handle)) = (&manifest, ctx.opt_config_handle())
                        {
                            match crate::config::managed::apply_policy_outcome_to_handle(
                                handle, manifest, &outcome,
                            ) {
                                Ok(()) => Ok(outcome),
                                Err(e) => Err(e),
                            }
                        } else {
                            Ok(outcome)
                        }
                    }
                    Err(e) => Err(e),
                };

                match result {
                    Ok(PolicyOutcome::Fresh(policy)) => {
                        print_refresh_outcome(ctx, &format, "fresh", Some(&policy), None);
                        Ok(())
                    }
                    Ok(PolicyOutcome::FromCache { policy, stale }) => {
                        print_refresh_outcome(
                            ctx,
                            &format,
                            "from_cache",
                            Some(&policy),
                            Some(stale),
                        );
                        Ok(())
                    }
                    Ok(PolicyOutcome::Unmanaged) => {
                        print_refresh_outcome(ctx, &format, "unmanaged", None, None);
                        Ok(())
                    }
                    Ok(PolicyOutcome::Denied) => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: CFG003,
                            category: DiagnosticCategory::Validation,
                            message: "access denied (unauthorized or forbidden); refusing to \
                                      fall back to any cached policy"
                                .to_string(),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("CFG003"))
                    }
                    Err(e) => {
                        DiagnosticReporter::report(&Diagnostic {
                            code: CFG003,
                            category: DiagnosticCategory::Validation,
                            message: format!("policy refresh failed: {e}"),
                            suggestion: None,
                            span: None,
                        });
                        Err(anyhow::anyhow!("CFG003"))
                    }
                }
            })
        }),
    }
}

fn print_refresh_outcome(
    ctx: &mut dyn AppContext,
    format: &str,
    outcome: &str,
    policy: Option<&crate::config::Policy>,
    stale: Option<bool>,
) {
    if format == "json" {
        let payload = serde_json::json!({
            "outcome": outcome,
            "profile": policy.map(|p| p.profile.clone()),
            "policy_version": policy.map(|p| p.policy_version),
            "stale": stale,
        });
        ctx.framework_println(&serde_json::to_string(&payload).unwrap_or_default());
        return;
    }
    let line = match (outcome, policy, stale) {
        ("fresh", Some(p), _) => format!(
            "Refreshed: profile '{}' at policy version {}",
            p.profile, p.policy_version
        ),
        ("from_cache", Some(p), Some(true)) => format!(
            "Server unreachable; using STALE cached policy (profile '{}', version {})",
            p.profile, p.policy_version
        ),
        ("from_cache", Some(p), _) => format!(
            "Server unreachable; using cached policy (profile '{}', version {})",
            p.profile, p.policy_version
        ),
        ("unmanaged", ..) => "this identity is not managed for this app; running unmanaged \
                              (any cached policy was cleared)"
            .to_string(),
        _ => outcome.to_string(),
    };
    ctx.framework_println(&line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
    use crate::config::resolution::{resolve, Provenance};
    use serde_json::json;

    // Small local fixture mirroring the one in `config::resolution::resolver`'s
    // own tests (spec 021 house convention: each test file/module rebuilds
    // its own tiny fixture rather than importing another test binary's
    // private helpers).
    fn field(key: &str, kind: FieldKind, default: serde_json::Value) -> FieldManifest {
        FieldManifest {
            key: key.to_string(),
            kind,
            default: Some(default),
            label: None,
            description: None,
            group: None,
            scope: Scope::Machine,
            platforms: vec![],
            secret: false,
            local_only: false,
            protected: false,
            manageable: true,
            enforceable: true,
            restart_required: false,
            constraints: None,
        }
    }

    #[test]
    fn layer_label_matches_serde_rename() {
        assert_eq!(layer_label(Layer::Default), "default");
        assert_eq!(layer_label(Layer::Recommended), "recommended");
        assert_eq!(layer_label(Layer::ConfigFile), "config_file");
        assert_eq!(layer_label(Layer::Environment), "environment");
        assert_eq!(layer_label(Layer::Flags), "flags");
        assert_eq!(layer_label(Layer::BuilderOverride), "builder_override");
        assert_eq!(layer_label(Layer::Enforced), "enforced");
    }

    #[test]
    fn display_value_unquotes_strings_but_not_other_kinds() {
        assert_eq!(display_value(&json!("hello")), "hello");
        assert_eq!(display_value(&json!(42)), "42");
        assert_eq!(display_value(&json!(true)), "true");
        assert_eq!(display_value(&json!(["a", "b"])), r#"["a","b"]"#);
    }

    #[test]
    fn format_value_falls_back_to_default_for_missing_or_wrong_type() {
        let mut args = HashMap::new();
        assert_eq!(format_value(&args, "table"), "table");
        args.insert("format".to_string(), ArgValue::Bool(true));
        assert_eq!(format_value(&args, "table"), "table");
        args.insert("format".to_string(), ArgValue::Enum("json".to_string()));
        assert_eq!(format_value(&args, "table"), "json");
    }

    fn sample_entries() -> Vec<ResolvedEntry> {
        // Reuses the resolver's own `resolve()` entry point (spec 021
        // testing decisions: assert observable resolution outcomes) rather
        // than hand-building `ResolvedEntry` values directly.
        let manifest = ConfigManifest::new(
            "app",
            vec![
                field("greeting", FieldKind::Str, json!("hello")),
                field("retries", FieldKind::Int, json!(3)),
            ],
        );
        let mut input = ResolutionInput::default();
        input.enforced.insert("retries".to_string(), json!(9));
        let resolved = resolve(&manifest, &input);
        let mut entries = resolved.entries();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    #[test]
    fn render_show_table_lists_every_field_with_layer_and_lock_state() {
        let entries = sample_entries();
        let table = render_show_table(&entries).unwrap();
        assert!(table.contains("greeting"));
        assert!(table.contains("hello"));
        assert!(table.contains("default"));
        assert!(table.contains("retries"));
        assert!(table.contains("9"));
        assert!(table.contains("enforced"));
        // Header row present.
        assert!(table.contains("Field"));
        assert!(table.contains("Locked"));
    }

    #[test]
    fn render_show_json_round_trips_field_value_layer_locked() {
        let entries = sample_entries();
        let rendered = render_show_json(&entries).unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let array = value.as_array().unwrap();
        assert_eq!(array.len(), 2);

        let greeting = array.iter().find(|e| e["field"] == "greeting").unwrap();
        assert_eq!(greeting["value"], json!("hello"));
        assert_eq!(greeting["layer"], json!("default"));
        assert_eq!(greeting["locked"], json!(false));

        let retries = array.iter().find(|e| e["field"] == "retries").unwrap();
        assert_eq!(retries["value"], json!(9));
        assert_eq!(retries["layer"], json!("enforced"));
        assert_eq!(retries["locked"], json!(true));
    }

    /// A context that overrides nothing — every `AppContext` accessor
    /// returns its default (`None`). Used to reach the `report_no_manifest`
    /// / `CFG001` branch directly: it is otherwise unreachable through a
    /// real CLI invocation, since the `config` group is only ever
    /// auto-registered once a manifest has been declared (see
    /// `AppBuilder::build`'s guard).
    struct NoManifestCtx;
    impl AppContext for NoManifestCtx {}

    #[tokio::test]
    async fn show_without_a_registered_manifest_reports_cfg001() {
        let mut ctx = NoManifestCtx;
        let cmd = build_show_command();
        let result = (cmd.execute)(&mut ctx, HashMap::new()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn manifest_without_a_registered_manifest_reports_cfg001() {
        let mut ctx = NoManifestCtx;
        let cmd = build_manifest_command();
        let result = (cmd.execute)(&mut ctx, HashMap::new()).await;
        assert!(result.is_err());
    }

    #[test]
    fn resolved_entry_provenance_is_reachable_from_show_rendering() {
        // Sanity that `Resolved::entries()` (the new resolver convenience
        // this command relies on) actually carries `Provenance`, not just a
        // layer label — belt-and-braces given `render_show_json` only reads
        // `.provenance.layer`/`.provenance.locked` off it.
        let entries = sample_entries();
        let retries = entries.iter().find(|e| e.path == "retries").unwrap();
        assert_eq!(
            retries.provenance,
            Provenance {
                layer: Layer::Enforced,
                locked: true,
            }
        );
    }
}
