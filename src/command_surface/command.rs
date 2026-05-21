use crate::command::Command;
use crate::command_surface::collect::collect;
use crate::command_surface::render::{render_json, render_markdown, render_yaml};
use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::E_UNSUPPORTED_SHELL;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::CommandSpec;
use crate::spec::value::ArgValue;
use std::sync::Arc;

/// Returns the built-in `spec` Command for auto-registration in AppBuilder::build.
pub fn create_spec_command(app_name: &'static str, app_version: &'static str) -> Command {
    Command {
        id: "spec",
        summary: "Export the CLI command surface as JSON, YAML, or Markdown",
        syntax: Some("spec [--format <json|yaml|markdown>] [--output <path>] [--include-hidden]"),
        category: None,
        spec: Some(Arc::new(spec_spec())),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(move |ctx, args| {
            let format_str = args
                .named
                .get("format")
                .cloned()
                .unwrap_or_else(|| "json".to_string());
            let output_path = args.named.get("output").cloned();
            let include_hidden = args
                .named
                .get("include-hidden")
                .map(|v| v == "true")
                .unwrap_or(false);

            // Access the registry synchronously before entering the async block
            // so we don't need to hold a reference across an await boundary.
            let doc = ctx
                .opt_registry()
                .map(|reg| collect(reg, app_name, app_version, include_hidden));

            Box::pin(async move {
                if format_str != "json" && format_str != "yaml" && format_str != "markdown" {
                    return Err(anyhow::anyhow!(
                        "CS001: unknown format '{}'; expected json, yaml, or markdown",
                        format_str
                    ));
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
pub fn create_completion_command(app_name: &'static str) -> Command {
    Command {
        id: "completion",
        summary: "Emit a shell completion stub for top-level subcommands",
        syntax: Some("completion <bash|zsh|fish|powershell|pwsh>"),
        category: None,
        spec: Some(Arc::new(completion_spec())),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(move |ctx, args| {
            let shell_token = args.named.get("shell").cloned().unwrap_or_default();

            let shell = match shell_token.as_str() {
                "bash" => Some(crate::app::Shell::Bash),
                "zsh" => Some(crate::app::Shell::Zsh),
                "fish" => Some(crate::app::Shell::Fish),
                "powershell" | "pwsh" => Some(crate::app::Shell::PowerShell),
                _ => None,
            };

            let registry = ctx
                .opt_registry()
                .expect("completion requires registry exposure");

            Box::pin(async move {
                let Some(shell) = shell else {
                    crate::app::diagnostic_reporter::DiagnosticReporter::report(&Diagnostic {
                        code: E_UNSUPPORTED_SHELL,
                        category: DiagnosticCategory::Parse,
                        message: format!(
                            "unsupported shell '{}'; expected bash, zsh, fish, powershell, or pwsh",
                            shell_token
                        ),
                        suggestion: None,
                        span: None,
                    });
                    return Err(anyhow::anyhow!("completion: unsupported shell"));
                };

                let cmds = crate::app::builder::visible_top_level_commands(registry);
                let mut stdout = std::io::stdout();
                crate::app::builder::emit_completion_script(app_name, shell, &cmds, &mut stdout)?;
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
                default: Some(ArgValue::Str("json".to_string())),
                conflicts_with: vec![],
                requires: vec![],
                help: "Output format: json, yaml, or markdown (default: json)",
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
        }],
        hidden_aliases: vec!["completions"],
        ..Default::default()
    }
}
