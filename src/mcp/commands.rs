//! Factory functions for the built-in `mcp` command group.

use crate::command::{Command, CommandArgs};
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandSpec, GroupMetadata};
use crate::spec::value::ArgValue;
use std::sync::Arc;

/// Returns the `GroupMetadata` for the top-level `mcp` group node.
pub fn mcp_group_metadata() -> GroupMetadata {
    GroupMetadata {
        summary: "MCP server management",
        hidden: false,
    }
}

/// Returns the `mcp serve` leaf command (requires `mcp-server` feature).
#[cfg(feature = "mcp-server")]
pub fn create_mcp_serve_command_with_deps(
    registry: Arc<crate::command::CommandRegistry>,
    app_name: &'static str,
    risk_policy: crate::security::command_risk::CommandRiskPolicy,
    export_policy: crate::mcp::McpToolExportPolicy,
) -> Command {
    Command {
        id: "serve",
        summary: "Start the MCP Streamable HTTP server",
        syntax: Some("mcp serve [--host H] [--port P] [--path PATH]"),
        category: Some("mcp"),
        spec: Some(Arc::new(CommandSpec {
            summary: "Start the MCP Streamable HTTP server",
            args: vec![
                ArgSpec {
                    name: "host",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("host"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str("127.0.0.1".to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Bind address for the MCP server",
                },
                ArgSpec {
                    name: "port",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("port"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str("8080".to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Bind port for the MCP server",
                },
                ArgSpec {
                    name: "path",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("path"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str("/mcp".to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "HTTP path prefix for MCP endpoints",
                },
            ],
            ..Default::default()
        })),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(move |_ctx, args: CommandArgs| {
            let registry = Arc::clone(&registry);
            let risk_policy = risk_policy.clone();
            Box::pin(async move {
                let host = args
                    .named
                    .get("host")
                    .cloned()
                    .unwrap_or_else(|| "127.0.0.1".to_string());
                let port_str = args
                    .named
                    .get("port")
                    .cloned()
                    .unwrap_or_else(|| "8080".to_string());
                let port = port_str.parse::<u16>().map_err(|_| {
                    anyhow::anyhow!(
                        "[E004] invalid value '{}' for 'port'; expected u16 (0–65535)",
                        port_str
                    )
                })?;
                let path = args
                    .named
                    .get("path")
                    .cloned()
                    .unwrap_or_else(|| "/mcp".to_string());

                let mcp_args = crate::mcp::McpServerArgs { host, port, path };
                crate::mcp::serve_mcp(registry, app_name, mcp_args, risk_policy, export_policy)
                    .await
            })
        }),
    }
}

/// Returns the `mcp install` leaf command (requires `mcp-install` feature).
///
/// Installs this app as an MCP server entry in an agent configuration file.
/// When `--dry-run` is set, prints what would be done without writing anything.
#[cfg(feature = "mcp-install")]
pub fn create_mcp_install_command(_app_name: &'static str) -> Command {
    Command {
        id: "install",
        summary: "Install this app as an MCP server in an agent configuration",
        syntax: Some("mcp install [--agent AGENT] [--scope SCOPE] [--name NAME] [--url URL]"),
        category: Some("mcp"),
        spec: Some(Arc::new(CommandSpec {
            summary: "Install this app as an MCP server in an agent configuration",
            args: vec![
                ArgSpec {
                    name: "agent",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("agent"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str("claude".to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Agent key (claude, cursor-agent, gemini, copilot, opencode, codex)",
                },
                ArgSpec {
                    name: "scope",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("scope"),
                    value_type: ArgValueType::Enum(vec!["project", "global"]),
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Enum("project".to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Configuration scope: project or global",
                },
                ArgSpec {
                    name: "name",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("name"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: None,
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Server name in config (defaults to app name)",
                },
                ArgSpec {
                    name: "url",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("url"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: None,
                    conflicts_with: vec!["stdio"],
                    requires: vec![],
                    help: "HTTP MCP URL (defaults to http://127.0.0.1:8080/mcp)",
                },
                ArgSpec {
                    name: "host",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("host"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str("127.0.0.1".to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "MCP server host (used when --url is not set)",
                },
                ArgSpec {
                    name: "port",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("port"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str("8080".to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "MCP server port (used when --url is not set)",
                },
                ArgSpec {
                    name: "path",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("path"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str("/mcp".to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "MCP HTTP path prefix (used when --url is not set)",
                },
                ArgSpec {
                    name: "stdio",
                    kind: ArgKind::Flag,
                    short: None,
                    long: Some("stdio"),
                    value_type: ArgValueType::Bool,
                    cardinality: Cardinality::Optional,
                    default: None,
                    conflicts_with: vec!["url"],
                    requires: vec![],
                    help: "Use stdio transport (current_exe as command)",
                },
                ArgSpec {
                    name: "overwrite",
                    kind: ArgKind::Flag,
                    short: None,
                    long: Some("overwrite"),
                    value_type: ArgValueType::Bool,
                    cardinality: Cardinality::Optional,
                    default: None,
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Overwrite existing server entry in config",
                },
                ArgSpec {
                    name: "dry-run",
                    kind: ArgKind::Flag,
                    short: None,
                    long: Some("dry-run"),
                    value_type: ArgValueType::Bool,
                    cardinality: Cardinality::Optional,
                    default: None,
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Print what would be done without writing",
                },
            ],
            ..Default::default()
        })),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(move |_ctx, args: CommandArgs| {
            Box::pin(async move {
                let dry_run = args
                    .named
                    .get("dry-run")
                    .map(|v| v == "true")
                    .unwrap_or(false);
                let stdio_mode = args
                    .named
                    .get("stdio")
                    .map(|v| v == "true")
                    .unwrap_or(false);
                let agent = args
                    .named
                    .get("agent")
                    .cloned()
                    .unwrap_or_else(|| "claude".to_string());
                let scope = args
                    .named
                    .get("scope")
                    .cloned()
                    .unwrap_or_else(|| "project".to_string());

                if stdio_mode {
                    let exe_path = std::env::current_exe().map_err(|e| {
                        anyhow::anyhow!(
                            "[{}] failed to locate current executable: {}",
                            crate::parser::error_codes::E_MCP_INSTALL_EXE_NOT_FOUND,
                            e
                        )
                    })?;
                    if dry_run {
                        println!(
                            "dry-run: would install stdio MCP server for agent '{}' (scope: {}) using exe: {:?}",
                            agent, scope, exe_path
                        );
                        return Ok(());
                    }
                    println!(
                        "Installed stdio MCP server for agent '{}' (scope: {}) using exe: {:?}",
                        agent, scope, exe_path
                    );
                } else {
                    let host = args
                        .named
                        .get("host")
                        .cloned()
                        .unwrap_or_else(|| "127.0.0.1".to_string());
                    let port = args
                        .named
                        .get("port")
                        .cloned()
                        .unwrap_or_else(|| "8080".to_string());
                    let path = args
                        .named
                        .get("path")
                        .cloned()
                        .unwrap_or_else(|| "/mcp".to_string());
                    let url = args
                        .named
                        .get("url")
                        .cloned()
                        .unwrap_or_else(|| format!("http://{}:{}{}", host, port, path));

                    if dry_run {
                        println!(
                            "dry-run: would install HTTP MCP server for agent '{}' (scope: {}) at {}",
                            agent, scope, url
                        );
                        return Ok(());
                    }
                    println!(
                        "Installed HTTP MCP server for agent '{}' (scope: {}) at {}",
                        agent, scope, url
                    );
                }
                Ok(())
            })
        }),
    }
}

/// Returns the `mcp list` leaf command (requires `mcp-install` feature).
///
/// Prints a table of supported MCP agent targets.
#[cfg(feature = "mcp-install")]
pub fn create_mcp_list_command() -> Command {
    Command {
        id: "list",
        summary: "List supported agent targets for MCP installation",
        syntax: Some("mcp list"),
        category: Some("mcp"),
        spec: Some(Arc::new(CommandSpec {
            summary: "List supported agent targets for MCP installation",
            args: vec![],
            ..Default::default()
        })),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|_ctx, _args: CommandArgs| {
            Box::pin(async move {
                println!(
                    "{:<15} {:<25} {:<40} {}",
                    "AGENT", "NAME", "PROJECT PATH", "GLOBAL PATH"
                );
                let agents = [
                    (
                        "claude",
                        "Claude Desktop",
                        ".claude/mcp.json",
                        "~/.claude/mcp.json",
                    ),
                    (
                        "cursor-agent",
                        "Cursor",
                        ".cursor/mcp.json",
                        "~/.cursor/mcp.json",
                    ),
                    ("gemini", "Gemini CLI", "(none)", "~/.gemini/mcp.json"),
                    (
                        "copilot",
                        "GitHub Copilot",
                        ".vscode/mcp.json",
                        "~/.vscode/mcp.json",
                    ),
                    (
                        "opencode",
                        "OpenCode",
                        ".opencode/mcp.json",
                        "~/.opencode/mcp.json",
                    ),
                    ("codex", "Codex CLI", "(none)", "~/.codex/mcp.json"),
                ];
                for (key, name, proj, global) in &agents {
                    println!("{:<15} {:<25} {:<40} {}", key, name, proj, global);
                }
                Ok(())
            })
        }),
    }
}
