//! Clap-based argv parsing adapter.
//!
//! **Design deviation from spec Section 4.3:** The spec suggests using
//! `sub_matches.ids()` + `get_many::<String>()` to extract parsed args from
//! Clap. However, because commands are registered dynamically at runtime and
//! their accepted flags/args are not known at build time, we cannot register
//! individual named arguments with Clap. Instead, each subcommand uses a
//! `trailing_var_arg` to capture all remaining args, which are then
//! classified as named (`--key value` / `--key=value`) or positional in
//! `match_to_command_args`. This preserves Clap's handling of `--help`,
//! `--version`, `--` terminator, and subcommand routing while accommodating
//! the dynamic command model.
//!
//! **Design deviation from spec Section 5.1:** `build_clap_root` accepts
//! `app_name` and `app_version` as separate parameters in addition to
//! `meta: Option<&AppMeta>`, because `App` stores these independently of
//! `AppMeta` (fields `app_name` / `app_version` on the `App` struct). When
//! `meta` is `None`, the name and version still need to be propagated to
//! Clap.

use crate::app::AppMeta;
use crate::command::{CommandArgs, CommandRegistry};
use crate::parser::clap_mapper::{
    build_legacy_clap_command, build_typed_clap_command, map_matches_to_typed_args,
};
use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::{E_UNKNOWN_COMMAND, E_UNKNOWN_FLAG};
use crate::parser::outcome::ParseOutcome;
use crate::spec::command_tree::CommandPath;

pub struct ParsedCommand {
    pub command_id: String,
    pub args: CommandArgs,
}

pub fn build_clap_root(
    meta: Option<&AppMeta>,
    registry: &CommandRegistry,
    app_name: &'static str,
    app_version: &'static str,
) -> clap::Command {
    let name = meta.map(|m| m.name).unwrap_or(app_name);
    let version = meta.map(|m| m.version).unwrap_or(app_version);

    let mut root = clap::Command::new(name)
        .version(version)
        .propagate_version(true)
        .subcommand_required(true)
        .arg_required_else_help(true);

    if let Some(m) = meta {
        if !m.description.is_empty() {
            root = root.about(m.description);
        }
        if let Some(usage) = m.usage {
            root = root.override_usage(usage);
        }
    }

    for cmd in registry.commands() {
        let sub = if let Some(ref spec) = cmd.spec {
            build_typed_clap_command(cmd.id, spec)
        } else {
            build_legacy_clap_command(cmd)
        };
        root = root.subcommand(sub);
    }

    let has_version_cmd = registry.get("version").is_some();
    if !has_version_cmd {
        root = root.subcommand(
            clap::Command::new("version")
                .about("Print version information")
                .arg(
                    clap::Arg::new("trailing")
                        .num_args(0..)
                        .trailing_var_arg(true)
                        .allow_hyphen_values(true),
                ),
        );
    }

    #[cfg(feature = "mcp-server")]
    {
        root = root
            .arg(
                clap::Arg::new("mcp-serve")
                    .long("mcp-serve")
                    .action(clap::ArgAction::SetTrue)
                    .global(true)
                    .help("Enable MCP server mode (Streamable HTTP)"),
            )
            .arg(
                clap::Arg::new("mcp-host")
                    .long("mcp-host")
                    .value_name("HOST")
                    .default_value("127.0.0.1")
                    .global(true)
                    .help("MCP server bind address"),
            )
            .arg(
                clap::Arg::new("mcp-port")
                    .long("mcp-port")
                    .value_name("PORT")
                    .value_parser(clap::value_parser!(u16))
                    .default_value("8080")
                    .global(true)
                    .help("MCP server bind port"),
            )
            .arg(
                clap::Arg::new("mcp-path")
                    .long("mcp-path")
                    .value_name("PATH")
                    .default_value("/mcp")
                    .global(true)
                    .help("MCP HTTP path prefix"),
            );
    }

    root
}

#[cfg(feature = "mcp-server")]
#[derive(Debug, Default)]
pub struct McpGlobalFlags {
    pub mcp_serve: bool,
    pub mcp_host: String,
    pub mcp_port: u16,
    pub mcp_path: String,
}

#[cfg(feature = "mcp-server")]
pub fn extract_mcp_flags(matches: &clap::ArgMatches) -> McpGlobalFlags {
    McpGlobalFlags {
        mcp_serve: matches.get_flag("mcp-serve"),
        mcp_host: matches
            .get_one::<String>("mcp-host")
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        mcp_port: *matches.get_one::<u16>("mcp-port").unwrap_or(&8080),
        mcp_path: matches
            .get_one::<String>("mcp-path")
            .cloned()
            .unwrap_or_else(|| "/mcp".to_string()),
    }
}

/// Parse argv against the clap command tree, returning a typed `ParseOutcome`.
///
/// - `HelpShown` / `VersionShown`: clap wrote to stdout; caller should return `Ok(())`.
/// - `ParseError(d)`: a structured diagnostic; caller should report it and return `Ok(())`.
/// - `Parsed { .. }`: a matched command; caller should dispatch execution.
pub fn parse_with_clap(
    root: &clap::Command,
    registry: &CommandRegistry,
    args: Vec<String>,
) -> ParseOutcome {
    match root.clone().try_get_matches_from(&args) {
        Ok(matches) => {
            let (name, sub_matches) = matches.subcommand().expect("subcommand required");
            let command_path = CommandPath::root_for(name);

            let (cmd_args, typed_args) = if let Some(cmd) = registry.get(name) {
                if let Some(ref spec) = cmd.spec {
                    match map_matches_to_typed_args(spec, sub_matches) {
                        Ok(typed) => (CommandArgs::default(), Some(typed)),
                        Err(d) => return ParseOutcome::ParseError(d),
                    }
                } else {
                    (match_to_command_args(sub_matches), None)
                }
            } else {
                // Built-in commands (e.g. "version") not in the user registry
                (match_to_command_args(sub_matches), None)
            };

            ParseOutcome::Parsed {
                command_path,
                args: cmd_args,
                typed_args,
            }
        }
        Err(e) => {
            use clap::error::ErrorKind;
            match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    // Stage 2: Parser is side-effect-free; return text for app layer to print
                    ParseOutcome::HelpShown(e.to_string())
                }
                ErrorKind::DisplayVersion => {
                    // Stage 2: Parser is side-effect-free; return text for app layer to print
                    ParseOutcome::VersionShown(e.to_string())
                }
                ErrorKind::UnknownArgument => ParseOutcome::ParseError(Diagnostic {
                    code: E_UNKNOWN_FLAG,
                    category: DiagnosticCategory::Parse,
                    message: "unknown argument".to_string(),
                    suggestion: Some("Use --help to see available arguments".to_string()),
                    span: None,
                }),
                _ => {
                    let cmd_arg = args.get(1).cloned().unwrap_or_default();
                    ParseOutcome::ParseError(Diagnostic {
                        code: E_UNKNOWN_COMMAND,
                        category: DiagnosticCategory::Parse,
                        message: format!("unrecognized subcommand '{}'", cmd_arg),
                        suggestion: Some("Use --help to see available commands".to_string()),
                        span: Some(cmd_arg),
                    })
                }
            }
        }
    }
}

fn match_to_command_args(sub_matches: &clap::ArgMatches) -> CommandArgs {
    let mut positional = Vec::new();
    let mut named = std::collections::HashMap::new();

    if let Some(values) = sub_matches.get_many::<String>("trailing") {
        let args: Vec<&str> = values.map(|s| s.as_str()).collect();
        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg.starts_with("--") {
                let stripped = &arg[2..];
                if stripped.is_empty() {
                    // Bare "--" after Clap's terminator: remaining items are positional.
                    i += 1;
                    while i < args.len() {
                        positional.push(args[i].to_string());
                        i += 1;
                    }
                    break;
                }
                if let Some(eq_pos) = stripped.find('=') {
                    let key = &stripped[..eq_pos];
                    let value = &stripped[eq_pos + 1..];
                    named.insert(key.to_string(), value.to_string());
                    i += 1;
                } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                    named.insert(stripped.to_string(), args[i + 1].to_string());
                    i += 2;
                } else {
                    // DD#8: bare --flag without a value is treated as a boolean flag.
                    // CommandArgs.named is HashMap<String, String> which cannot represent
                    // a boolean. Per the spec, we do NOT insert "true" (correctness
                    // improvement). Apps needing boolean flags should use explicit flag
                    // args in future phases with Clap derive.
                    i += 1;
                }
            } else {
                positional.push(arg.to_string());
                i += 1;
            }
        }
    }

    CommandArgs { positional, named }
}
