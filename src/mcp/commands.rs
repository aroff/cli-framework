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
    gate: Option<std::sync::Arc<dyn crate::mcp::McpToolGate>>,
) -> Command {
    Command {
        id: "serve",
        summary: "Start the MCP server (http or stdio)",
        syntax: Some("mcp serve [--transport http|stdio] [--host H] [--port P] [--path PATH]"),
        category: Some("mcp"),
        spec: Some(Arc::new(CommandSpec {
            summary: "Start the MCP server (http or stdio)",
            args: vec![
                ArgSpec {
                    name: "transport",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("transport"),
                    value_type: ArgValueType::Enum(vec!["http", "stdio"]),
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Enum("http".to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Transport: http (Streamable HTTP) or stdio (stdin/stdout JSON-RPC)",
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
            let gate = gate.clone();
            Box::pin(async move {
                let transport = args
                    .named
                    .get("transport")
                    .cloned()
                    .unwrap_or_else(|| "http".to_string());

                if transport == "stdio" {
                    if args.named.contains_key("host")
                        || args.named.contains_key("port")
                        || args.named.contains_key("path")
                    {
                        return Err(anyhow::anyhow!(
                            "[E004] invalid usage: '--host', '--port', and '--path' are only valid when --transport=http"
                        ));
                    }

                    return crate::mcp::serve_mcp_stdio(
                        registry,
                        app_name,
                        risk_policy,
                        export_policy,
                        gate,
                    )
                    .await;
                }

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
                crate::mcp::serve_mcp_with_gate(
                    registry,
                    app_name,
                    mcp_args,
                    risk_policy,
                    export_policy,
                    gate,
                )
                .await
            })
        }),
    }
}

/// Returns the `mcp install` leaf command (requires `mcp-install` feature).
///
/// Installs this app as an MCP server entry in an agent configuration file via
/// `aikit_sdk::add_mcp_server`. When `--dry-run` is set, prints what would be
/// done without writing anything.
#[cfg(feature = "mcp-install")]
pub fn create_mcp_install_command(app_name: &'static str) -> Command {
    Command {
        id: "install",
        summary: "Install this app as an MCP server in an agent configuration",
        syntax: Some(
            "mcp install [--agent AGENT] [--scope SCOPE] [--name NAME] [--url URL | --stdio]",
        ),
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
                    name: "project",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("project"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: None,
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Project root directory (default: current directory, for project scope)",
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
                    name: "header",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("header"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Repeated,
                    default: None,
                    conflicts_with: vec!["stdio"],
                    requires: vec![],
                    help: "HTTP header KEY=value (repeat for multiple; HTTP mode only)",
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
                    name: "arg",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("arg"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Repeated,
                    default: None,
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Additional argv token for stdio command (repeat for multiple)",
                },
                ArgSpec {
                    name: "env",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("env"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Repeated,
                    default: None,
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Environment variable KEY=value for stdio (repeat for multiple)",
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
                use crate::parser::error_codes::{
                    E_MCP_INSTALL_EXE_NOT_FOUND, E_MCP_INSTALL_WRITE_FAILED,
                };

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
                let scope_str = args
                    .named
                    .get("scope")
                    .cloned()
                    .unwrap_or_else(|| "project".to_string());
                let server_name = args
                    .named
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| app_name.to_string());
                let overwrite = args
                    .named
                    .get("overwrite")
                    .map(|v| v == "true")
                    .unwrap_or(false);

                let project_root = args
                    .named
                    .get("project")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
                    });
                let project_root = project_root.canonicalize().unwrap_or(project_root.clone());

                let scope = if scope_str == "global" {
                    aikit_sdk::McpScope::Global
                } else {
                    aikit_sdk::McpScope::Project
                };

                let agent_key = aikit_sdk::normalize_mcp_agent_key(&agent).to_string();

                // Helper: split a comma-joined repeated-arg string into a Vec<String>.
                // Repeated args with Cardinality::Repeated are joined with ',' by the
                // framework's typed-arg-to-CommandArgs conversion.
                let split_repeated = |raw: &str| -> Vec<String> {
                    if raw.is_empty() {
                        vec![]
                    } else {
                        raw.split(',').map(|s| s.to_string()).collect()
                    }
                };

                let transport = if stdio_mode {
                    let exe_path = std::env::current_exe().map_err(|e| {
                        anyhow::anyhow!(
                            "[{}] failed to locate current executable: {}",
                            E_MCP_INSTALL_EXE_NOT_FOUND,
                            e
                        )
                    })?;

                    let exe_args: Vec<String> = args
                        .named
                        .get("arg")
                        .map(|r| split_repeated(r))
                        .unwrap_or_default();
                    let exe_args = if exe_args.is_empty() {
                        vec![
                            "mcp".to_string(),
                            "serve".to_string(),
                            "--transport".to_string(),
                            "stdio".to_string(),
                        ]
                    } else {
                        exe_args
                    };

                    let env_pairs: Vec<String> = args
                        .named
                        .get("env")
                        .map(|r| split_repeated(r))
                        .unwrap_or_default();
                    let env_map = if env_pairs.is_empty() {
                        None
                    } else {
                        Some(aikit_sdk::parse_env_pairs(&env_pairs).map_err(|e| {
                            anyhow::anyhow!(
                                "[{}] invalid --env value: {}",
                                E_MCP_INSTALL_WRITE_FAILED,
                                e
                            )
                        })?)
                    };

                    if dry_run {
                        println!(
                            "dry-run: would install stdio MCP server for agent '{}' (scope: {:?}) using exe: {} args: {:?}",
                            agent_key,
                            scope,
                            exe_path.display(),
                            exe_args
                        );
                        return Ok(());
                    }

                    aikit_sdk::McpServerTransport::Stdio {
                        command: exe_path.to_string_lossy().into_owned(),
                        args: exe_args,
                        env: env_map,
                    }
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

                    let header_pairs: Vec<String> = args
                        .named
                        .get("header")
                        .map(|r| split_repeated(r))
                        .unwrap_or_default();
                    let headers = if header_pairs.is_empty() {
                        None
                    } else {
                        Some(aikit_sdk::parse_header_pairs(&header_pairs).map_err(|e| {
                            anyhow::anyhow!(
                                "[{}] invalid --header value: {}",
                                E_MCP_INSTALL_WRITE_FAILED,
                                e
                            )
                        })?)
                    };

                    if dry_run {
                        println!(
                            "dry-run: would install HTTP MCP server for agent '{}' (scope: {:?}) at {}",
                            agent_key, scope, url
                        );
                        return Ok(());
                    }

                    aikit_sdk::McpServerTransport::Http { url, headers }
                };

                let opts = aikit_sdk::AddMcpServerOptions {
                    agent_key,
                    scope,
                    project_root,
                    server_name,
                    transport,
                    overwrite,
                };

                let written_path =
                    tokio::task::spawn_blocking(move || aikit_sdk::add_mcp_server(opts))
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "[{}] internal error: {}",
                                E_MCP_INSTALL_WRITE_FAILED,
                                e
                            )
                        })?
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "[{}] failed to install MCP server: {}",
                                E_MCP_INSTALL_WRITE_FAILED,
                                e
                            )
                        })?;

                println!("MCP server installed to: {}", written_path.display());
                Ok(())
            })
        }),
    }
}

/// Returns the `mcp list` leaf command (requires `mcp-install` feature).
///
/// Prints a table of supported MCP agent targets from `aikit_sdk::mcp_supported_agents()`.
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
                let agents = aikit_sdk::mcp_supported_agents();
                println!(
                    "{:<15} {:<25} {:<45} GLOBAL PATH",
                    "AGENT", "NAME", "PROJECT PATH"
                );
                for row in &agents {
                    println!(
                        "{:<15} {:<25} {:<45} {}",
                        row.agent_key,
                        row.display_name,
                        row.project_config_path,
                        row.global_config_path
                    );
                }
                Ok(())
            })
        }),
    }
}
