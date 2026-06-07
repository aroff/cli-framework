use crate::command::Command;
use crate::command_surface::collect::collect;
use crate::command_surface::render::{render_json, render_markdown, render_yaml};
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::CommandSpec;
use crate::spec::value::ArgValue;
use std::sync::Arc;

/// Returns the built-in `spec` Command for auto-registration in AppBuilder::build.
pub fn create_spec_command(app_name: &'static str, app_version: &'static str) -> Command {
    Command {
        id: Arc::from("spec"),
        spec: Arc::new(spec_spec()),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        execute: Arc::new(move |ctx, args| {
            let format_str = args
                .get("format")
                .and_then(|v| {
                    if let crate::spec::value::ArgValue::Enum(s)
                    | crate::spec::value::ArgValue::Str(s) = v
                    {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "json".to_string());
            let output_path = args.get("output").and_then(|v| {
                if let crate::spec::value::ArgValue::Str(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            });
            let include_hidden = args
                .get("include-hidden")
                .and_then(|v| {
                    if let crate::spec::value::ArgValue::Bool(b) = v {
                        Some(*b)
                    } else {
                        None
                    }
                })
                .unwrap_or(false);

            // Access the registry synchronously before entering the async block
            // so we don't need to hold a reference across an await boundary.
            let doc = ctx
                .opt_registry()
                .map(|reg| collect(reg, app_name, app_version, include_hidden));

            Box::pin(async move {
                // R4a rejects invalid Enum values at parse time; this is a defensive
                // fallback for the legacy (non-typed) path only.
                if format_str != "json" && format_str != "yaml" && format_str != "markdown" {
                    use crate::app::diagnostic_reporter::DiagnosticReporter;
                    use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
                    use crate::parser::error_codes::E_UNKNOWN_SPEC_FORMAT;
                    DiagnosticReporter::report(&Diagnostic {
                        code: E_UNKNOWN_SPEC_FORMAT,
                        category: DiagnosticCategory::Validation,
                        message: format!(
                            "unknown format '{}'; expected json, yaml, or markdown",
                            format_str
                        ),
                        suggestion: Some("Use --format json, yaml, or markdown".to_string()),
                        span: Some(format_str.clone()),
                    });
                    return Err(anyhow::Error::new(crate::app::UsageError(format!(
                        "unknown format '{}'; expected json, yaml, or markdown",
                        format_str
                    ))));
                }

                let doc = doc.unwrap_or_else(|| {
                    crate::command_surface::collect::collect(
                        &crate::command::CommandRegistry::new(),
                        app_name,
                        app_version,
                        include_hidden,
                    )
                });

                let rendered: String = match format_str.as_str() {
                    "json" => render_json(&doc)?,
                    "yaml" => render_yaml(&doc)?,
                    "markdown" => render_markdown(&doc),
                    _ => unreachable!(),
                };

                if let Some(path) = output_path {
                    std::fs::write(&path, &rendered).map_err(|e| {
                        anyhow::anyhow!("CS002: failed to write to '{}': {}", path, e)
                    })?;
                } else {
                    use std::io::Write;

                    let mut stdout = std::io::stdout();
                    writeln!(stdout, "{}", rendered)?;
                }

                Ok(())
            })
        }),
    }
}

/// Returns the built-in `completion` Command for auto-registration in AppBuilder::build.
///
/// The `clap_root` is captured in the execute closure so shell completion scripts can be
/// generated without requiring a reference to `App`.
pub fn create_completion_command(
    app_name: &'static str,
    clap_root: std::sync::Arc<clap::Command>,
) -> Command {
    Command {
        id: Arc::from("completion"),
        spec: Arc::new(completion_spec()),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        execute: Arc::new(move |ctx, args| {
            let _clap_root = std::sync::Arc::clone(&clap_root);
            Box::pin(async move {
                use crate::app::builder::{
                    emit_completion_script, visible_top_level_commands, Shell,
                };

                // R4a (Enum validation) rejects invalid shell values at parse time,
                // so shell_token is always a valid value when this closure runs.
                let shell_token = args
                    .get("shell")
                    .and_then(|v| match v {
                        crate::spec::value::ArgValue::Enum(s)
                        | crate::spec::value::ArgValue::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();

                let shell = match shell_token.as_str() {
                    "bash" => Shell::Bash,
                    "zsh" => Shell::Zsh,
                    "fish" => Shell::Fish,
                    "powershell" | "pwsh" => Shell::PowerShell,
                    other => {
                        // Defensive fallback; should not be reached after R4a.
                        return Err(anyhow::anyhow!(
                            "E013: unsupported shell '{}'; expected bash, zsh, fish, powershell, or pwsh",
                            other
                        ));
                    }
                };

                let registry = ctx.opt_registry();
                let cmds = if let Some(reg) = registry {
                    visible_top_level_commands(reg)
                } else {
                    std::collections::BTreeSet::new()
                };

                // Render to an in-memory buffer and emit via framework_println so
                // the script is captured by the app's per-instance stdout buffer
                // (testkit / API hosts) rather than written straight to fd 1. Writing
                // to fd 1 forced tests into process-global dup2 capture, which races
                // with libtest's own status writes under parallel execution.
                let mut buf: Vec<u8> = Vec::new();
                emit_completion_script(app_name, shell, &cmds, &mut buf)?;
                let script = String::from_utf8(buf)
                    .map_err(|e| anyhow::anyhow!("completion script not valid UTF-8: {}", e))?;
                ctx.framework_println(script.trim_end());
                Ok(())
            })
        }),
    }
}

fn spec_spec() -> CommandSpec {
    CommandSpec {
        summary: "Export the CLI command surface as JSON, YAML, or Markdown",
        args: vec![
            ArgSpec {
                name: "format",
                kind: ArgKind::Option,
                short: None,
                long: None,
                value_type: ArgValueType::Enum(vec!["json", "yaml", "markdown"]),
                cardinality: Cardinality::Optional,
                default: Some(ArgValue::Enum("json".to_string())),
                conflicts_with: vec![],
                requires: vec![],
                help: "Output format: json, yaml, or markdown (default: json)",
                ..Default::default()
            },
            ArgSpec {
                name: "output",
                kind: ArgKind::Option,
                short: None,
                long: None,
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Write output to this file path instead of stdout",
                ..Default::default()
            },
            ArgSpec {
                name: "include-hidden",
                kind: ArgKind::Flag,
                short: None,
                long: None,
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Include commands with hidden: true",
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn completion_spec() -> CommandSpec {
    CommandSpec {
        summary: "Emit a shell completion stub for top-level subcommands",
        args: vec![ArgSpec {
            name: "shell",
            kind: ArgKind::Positional,
            short: None,
            long: None,
            value_type: ArgValueType::Enum(vec!["bash", "zsh", "fish", "powershell", "pwsh"]),
            cardinality: Cardinality::Required,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "Target shell: bash, zsh, fish, powershell, or pwsh",
            ..Default::default()
        }],
        hidden_aliases: vec!["completions"],
        ..Default::default()
    }
}
