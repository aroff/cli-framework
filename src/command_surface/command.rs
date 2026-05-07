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
        id: "spec",
        summary: "Export the CLI command surface as JSON, YAML, or Markdown",
        syntax: Some("spec [--format <json|yaml|markdown>] [--output <path>] [--include-hidden]"),
        category: None,
        spec: Some(Arc::new(spec_spec())),
        validator: None,
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
                    println!("{}", rendered);
                }

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
