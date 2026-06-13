//! Factory functions for the built-in `mcp` command group.

const MCP_DEFAULT_HOST: &str = "127.0.0.1";
const MCP_DEFAULT_PORT: &str = "8080";
const MCP_DEFAULT_PATH: &str = "/mcp";

use crate::command::Command;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandSpec, GroupMetadata};
use crate::spec::value::ArgValue;
use std::collections::HashMap;
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
#[allow(clippy::too_many_arguments)]
pub fn create_mcp_serve_command_with_deps(
    registry: Arc<crate::command::CommandRegistry>,
    app_name: &'static str,
    risk_policy: crate::security::command_risk::CommandRiskPolicy,
    export_policy: crate::mcp::McpToolExportPolicy,
    gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
    resource_registry: Arc<crate::mcp::resources::ResourceRegistry>,
) -> Command {
    Command {
        id: Arc::from("serve"),
        spec: Arc::new(CommandSpec {
            summary: "Start the MCP server (http or stdio)",
            syntax: Some("mcp serve [--transport http|stdio] [--host H] [--port P] [--path PATH]"),
            category: Some("mcp"),
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
                    ..Default::default()
                },
                ArgSpec {
                    name: "host",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("host"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str(MCP_DEFAULT_HOST.to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Bind address for the MCP server",
                    ..Default::default()
                },
                ArgSpec {
                    name: "port",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("port"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str(MCP_DEFAULT_PORT.to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Bind port for the MCP server",
                    ..Default::default()
                },
                ArgSpec {
                    name: "path",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("path"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str(MCP_DEFAULT_PATH.to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "HTTP path prefix for MCP endpoints",
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(move |ctx, args: HashMap<String, ArgValue>| {
            let registry = Arc::clone(&registry);
            let risk_policy = risk_policy.clone();
            let gate = gate.clone();
            let resource_registry = Arc::clone(&resource_registry);
            // Resolve banner settings up front (ctx is not 'static, can't cross await).
            let banner = crate::mcp::BannerSettings::resolve(ctx.opt_global_args(), &args);
            Box::pin(async move {
                // Defaults injected by spec: transport="http", host="127.0.0.1", port="8080", path="/mcp"
                let transport = args
                    .get("transport")
                    .and_then(|v| match v {
                        ArgValue::Enum(s) | ArgValue::Str(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or("http");

                if transport == "stdio" {
                    // Check whether the user explicitly overrode the http-only defaults.
                    // After spec-default injection, these keys are always present; a value
                    // equal to the spec default means the user did not override it.
                    let host_overridden = args
                        .get("host")
                        .and_then(|v| match v {
                            ArgValue::Str(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .is_some_and(|v| v != MCP_DEFAULT_HOST);
                    let port_overridden = args
                        .get("port")
                        .and_then(|v| match v {
                            ArgValue::Str(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .is_some_and(|v| v != MCP_DEFAULT_PORT);
                    let path_overridden = args
                        .get("path")
                        .and_then(|v| match v {
                            ArgValue::Str(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .is_some_and(|v| v != MCP_DEFAULT_PATH);
                    if host_overridden || port_overridden || path_overridden {
                        return Err(anyhow::anyhow!(
                            "[E004] invalid usage: '--host', '--port', and '--path' are only valid when --transport=http"
                        ));
                    }
                    return crate::mcp::serve_mcp_stdio_opts_with_resources(
                        registry,
                        app_name,
                        risk_policy,
                        export_policy,
                        gate,
                        resource_registry,
                        banner,
                    )
                    .await;
                }

                let host = args
                    .get("host")
                    .and_then(|v| match v {
                        ArgValue::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| MCP_DEFAULT_HOST.to_string());
                let port_str = args
                    .get("port")
                    .and_then(|v| match v {
                        ArgValue::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| MCP_DEFAULT_PORT.to_string());
                let port = port_str.parse::<u16>().map_err(|_| {
                    anyhow::anyhow!(
                        "[E004] invalid value '{}' for 'port'; expected u16 (0–65535)",
                        port_str
                    )
                })?;
                let path = args
                    .get("path")
                    .and_then(|v| match v {
                        ArgValue::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| MCP_DEFAULT_PATH.to_string());

                crate::mcp::serve_mcp_with_gate_opts_with_resources(
                    registry,
                    app_name,
                    crate::mcp::McpServerArgs { host, port, path },
                    risk_policy,
                    export_policy,
                    gate,
                    resource_registry,
                    banner,
                )
                .await
            })
        }),
    }
}

#[cfg(feature = "mcp-install")]
struct McpInstallArgs {
    stdio_mode: bool,
    agent: String,
    scope: aikit_sdk::McpScope,
    server_name: String,
    overwrite: bool,
    project_root: std::path::PathBuf,
    url: Option<String>,
    host: String,
    port: String,
    path: String,
    headers: Option<std::collections::HashMap<String, String>>,
    exe_args: Vec<String>,
    env_map: Option<std::collections::HashMap<String, String>>,
}

#[cfg(feature = "mcp-install")]
fn parse_mcp_install_args(
    args: &HashMap<String, ArgValue>,
    app_name: &str,
) -> anyhow::Result<McpInstallArgs> {
    use crate::parser::error_codes::E_MCP_INSTALL_WRITE_FAILED;

    let stdio_mode = matches!(args.get("stdio"), Some(ArgValue::Bool(true)));
    let agent = args
        .get("agent")
        .and_then(|v| match v {
            ArgValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "claude".to_string());
    let scope_str = args
        .get("scope")
        .and_then(|v| match v {
            ArgValue::Enum(s) | ArgValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "project".to_string());
    let server_name = args
        .get("name")
        .and_then(|v| match v {
            ArgValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| app_name.to_string());
    let overwrite = matches!(args.get("overwrite"), Some(ArgValue::Bool(true)));

    let project_root = args
        .get("project")
        .and_then(|v| match v {
            ArgValue::Str(s) => Some(std::path::PathBuf::from(s)),
            _ => None,
        })
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });
    let project_root = project_root.canonicalize().unwrap_or(project_root);

    let scope = if scope_str == "global" {
        aikit_sdk::McpScope::Global
    } else {
        aikit_sdk::McpScope::Project
    };

    let host = args
        .get("host")
        .and_then(|v| match v {
            ArgValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| MCP_DEFAULT_HOST.to_string());
    let port = args
        .get("port")
        .and_then(|v| match v {
            ArgValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| MCP_DEFAULT_PORT.to_string());
    let path = args
        .get("path")
        .and_then(|v| match v {
            ArgValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| MCP_DEFAULT_PATH.to_string());
    let url = args.get("url").and_then(|v| match v {
        ArgValue::Str(s) => Some(s.clone()),
        _ => None,
    });

    let header_pairs: Vec<String> = match args.get("header") {
        Some(ArgValue::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                ArgValue::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => vec![],
    };
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

    let raw_exe_args: Vec<String> = match args.get("arg") {
        Some(ArgValue::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                ArgValue::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => vec![],
    };
    let exe_args = if raw_exe_args.is_empty() {
        vec![
            "mcp".to_string(),
            "serve".to_string(),
            "--transport".to_string(),
            "stdio".to_string(),
        ]
    } else {
        raw_exe_args
    };

    let env_pairs: Vec<String> = match args.get("env") {
        Some(ArgValue::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                ArgValue::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => vec![],
    };
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

    Ok(McpInstallArgs {
        stdio_mode,
        agent,
        scope,
        server_name,
        overwrite,
        project_root,
        url,
        host,
        port,
        path,
        headers,
        exe_args,
        env_map,
    })
}

#[cfg(feature = "mcp-install")]
fn dry_run_message(parsed: &McpInstallArgs) -> String {
    let agent_key = aikit_sdk::normalize_mcp_agent_key(&parsed.agent).to_string();
    if parsed.stdio_mode {
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "<unknown>".to_string());
        format!(
            "dry-run: would install stdio MCP server for agent '{}' (scope: {:?}) using exe: {} args: {:?}",
            agent_key, parsed.scope, exe, parsed.exe_args
        )
    } else {
        let url = parsed
            .url
            .clone()
            .unwrap_or_else(|| format!("http://{}:{}{}", parsed.host, parsed.port, parsed.path));
        format!(
            "dry-run: would install HTTP MCP server for agent '{}' (scope: {:?}) at {}",
            agent_key, parsed.scope, url
        )
    }
}

#[cfg(feature = "mcp-install")]
async fn run_mcp_install(parsed: McpInstallArgs) -> anyhow::Result<()> {
    use crate::parser::error_codes::{E_MCP_INSTALL_EXE_NOT_FOUND, E_MCP_INSTALL_WRITE_FAILED};

    let agent_key = aikit_sdk::normalize_mcp_agent_key(&parsed.agent).to_string();

    let transport = if parsed.stdio_mode {
        let exe_path = std::env::current_exe().map_err(|e| {
            anyhow::anyhow!(
                "[{}] failed to locate current executable: {}",
                E_MCP_INSTALL_EXE_NOT_FOUND,
                e
            )
        })?;
        aikit_sdk::McpServerTransport::Stdio {
            command: exe_path.to_string_lossy().into_owned(),
            args: parsed.exe_args,
            env: parsed.env_map,
        }
    } else {
        let url = parsed
            .url
            .unwrap_or_else(|| format!("http://{}:{}{}", parsed.host, parsed.port, parsed.path));
        aikit_sdk::McpServerTransport::Http {
            url,
            headers: parsed.headers,
        }
    };

    let opts = aikit_sdk::AddMcpServerOptions {
        agent_key,
        scope: parsed.scope,
        project_root: parsed.project_root,
        server_name: parsed.server_name,
        transport,
        overwrite: parsed.overwrite,
    };

    let written_path = tokio::task::spawn_blocking(move || aikit_sdk::add_mcp_server(opts))
        .await
        .map_err(|e| anyhow::anyhow!("[{}] internal error: {}", E_MCP_INSTALL_WRITE_FAILED, e))?
        .map_err(|e| {
            anyhow::anyhow!(
                "[{}] failed to install MCP server: {}",
                E_MCP_INSTALL_WRITE_FAILED,
                e
            )
        })?;

    println!("MCP server installed to: {}", written_path.display());
    Ok(())
}

/// Returns the `mcp install` leaf command (requires `mcp-install` feature).
#[cfg(feature = "mcp-install")]
pub fn create_mcp_install_command(app_name: &'static str) -> Command {
    Command {
        id: Arc::from("install"),
        spec: Arc::new(CommandSpec {
            summary: "Install this app as an MCP server in an agent configuration",
            syntax: Some(
                "mcp install [--agent AGENT] [--scope SCOPE] [--name NAME] [--url URL | --stdio]",
            ),
            category: Some("mcp"),
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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
                },
                ArgSpec {
                    name: "host",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("host"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str(MCP_DEFAULT_HOST.to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "MCP server host (used when --url is not set)",
                    ..Default::default()
                },
                ArgSpec {
                    name: "port",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("port"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str(MCP_DEFAULT_PORT.to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "MCP server port (used when --url is not set)",
                    ..Default::default()
                },
                ArgSpec {
                    name: "path",
                    kind: ArgKind::Option,
                    short: None,
                    long: Some("path"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    default: Some(ArgValue::Str(MCP_DEFAULT_PATH.to_string())),
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "MCP HTTP path prefix (used when --url is not set)",
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(move |ctx, args: HashMap<String, ArgValue>| {
            let dry_run = matches!(
                args.get("dry-run"),
                Some(crate::spec::value::ArgValue::Bool(true))
            );
            if dry_run {
                match parse_mcp_install_args(&args, app_name) {
                    Ok(parsed) => {
                        let normalized =
                            aikit_sdk::normalize_mcp_agent_key(&parsed.agent).to_string();
                        if !aikit_sdk::MCP_SUPPORTED_AGENT_KEYS.contains(&normalized.as_str()) {
                            let err_msg = format!(
                                "unknown agent key '{}'; supported: {}",
                                normalized,
                                aikit_sdk::MCP_SUPPORTED_AGENT_KEYS.join(", ")
                            );
                            return Box::pin(async move { Err(anyhow::anyhow!("{}", err_msg)) });
                        }
                        ctx.framework_println(&dry_run_message(&parsed));
                        return Box::pin(async move { Ok(()) });
                    }
                    Err(e) => {
                        return Box::pin(async move { Err(e) });
                    }
                }
            }
            Box::pin(async move {
                let parsed = parse_mcp_install_args(&args, app_name)?;
                run_mcp_install(parsed).await
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
        id: Arc::from("list"),
        spec: Arc::new(CommandSpec {
            summary: "List supported agent targets for MCP installation",
            syntax: Some("mcp list"),
            category: Some("mcp"),
            args: vec![],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(|ctx, _args: HashMap<String, ArgValue>| {
            let agents = aikit_sdk::mcp_supported_agents();
            let header = format!(
                "{:<15} {:<25} {:<45} GLOBAL PATH",
                "AGENT", "NAME", "PROJECT PATH"
            );
            ctx.framework_println(&header);
            let rows: Vec<String> = agents
                .iter()
                .map(|row| {
                    format!(
                        "{:<15} {:<25} {:<45} {}",
                        row.agent_key,
                        row.display_name,
                        row.project_config_path,
                        row.global_config_path
                    )
                })
                .collect();
            for row in &rows {
                ctx.framework_println(row);
            }
            Box::pin(async move { Ok(()) })
        }),
    }
}
