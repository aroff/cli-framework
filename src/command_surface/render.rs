use crate::command_surface::document::CliSpecDocument;

pub fn render_json(doc: &CliSpecDocument) -> anyhow::Result<String> {
    serde_json::to_string_pretty(doc)
        .map_err(|e| anyhow::anyhow!("CS004: JSON serialization error: {}", e))
}

pub fn render_yaml(doc: &CliSpecDocument) -> anyhow::Result<String> {
    serde_yaml::to_string(doc)
        .map_err(|e| anyhow::anyhow!("CS003: YAML serialization error: {}", e))
}

pub fn render_markdown(doc: &CliSpecDocument) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {} {}\n\n", doc.app.name, doc.app.version));

    for cmd in &doc.commands {
        out.push_str(&format!("## {}\n\n", cmd.path));
        out.push_str(&format!("{}\n\n", cmd.summary));

        if let Some(ref syntax) = cmd.syntax {
            out.push_str(&format!("**Syntax**: `{}`\n\n", syntax));
        }

        if let Some(ref category) = cmd.category {
            out.push_str(&format!("**Category**: {}\n\n", category));
        }

        if !cmd.args.is_empty() {
            out.push_str("| Name | Kind | Type | Cardinality | Default | Description |\n");
            out.push_str("|------|------|------|-------------|---------|-------------|\n");
            for arg in &cmd.args {
                let default = arg.default.as_deref().unwrap_or("");
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} |\n",
                    arg.name, arg.kind, arg.value_type, arg.cardinality, default, arg.help
                ));
            }
            out.push('\n');
        }

        if !cmd.examples.is_empty() {
            out.push_str("### Examples\n\n");
            for example in &cmd.examples {
                out.push_str("```\n");
                out.push_str(example);
                out.push_str("\n```\n\n");
            }
        }

        if !cmd.env_vars.is_empty() {
            out.push_str("### Environment Variables\n\n");
            out.push_str("| Variable | Description |\n");
            out.push_str("|----------|-------------|\n");
            for ev in &cmd.env_vars {
                out.push_str(&format!("| {} | {} |\n", ev.name, ev.description));
            }
            out.push('\n');
        }

        if !cmd.exit_codes.is_empty() {
            out.push_str("### Exit Codes\n\n");
            out.push_str("| Code | Description |\n");
            out.push_str("|------|-------------|\n");
            for ec in &cmd.exit_codes {
                out.push_str(&format!("| {} | {} |\n", ec.code, ec.description));
            }
            out.push('\n');
        }
    }

    out
}
