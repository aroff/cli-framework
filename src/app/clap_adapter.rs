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
use crate::parser::clap_mapper::{build_typed_clap_command, map_matches_to_typed_args};
use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::{E_NESTED_COMMAND_NOT_FOUND, E_UNKNOWN_COMMAND, E_UNKNOWN_FLAG};
use crate::parser::outcome::ParseOutcome;
use crate::spec::command_tree::CommandPath;

pub struct ParsedCommand {
    pub command_id: String,
    pub args: CommandArgs,
}

#[derive(Default)]
struct ClapTreeNode<'a> {
    command: Option<&'a crate::command::Command>,
    children: std::collections::BTreeMap<String, ClapTreeNode<'a>>,
}

impl<'a> ClapTreeNode<'a> {
    fn insert_command(&mut self, segments: &[&str], command: &'a crate::command::Command) {
        if segments.is_empty() {
            self.command = Some(command);
            return;
        }

        self.children
            .entry(segments[0].to_string())
            .or_default()
            .insert_command(&segments[1..], command);
    }

    fn insert_group(&mut self, segments: &[&str]) {
        if segments.is_empty() {
            return;
        }

        self.children
            .entry(segments[0].to_string())
            .or_default()
            .insert_group(&segments[1..]);
    }
}

pub fn build_clap_root(
    meta: Option<&AppMeta>,
    registry: &CommandRegistry,
    app_name: &'static str,
    app_version: &'static str,
    app_git_sha_short: Option<&'static str>,
) -> clap::Command {
    let name = meta.map(|m| m.name).unwrap_or(app_name);
    let version = meta.map(|m| m.version).unwrap_or(app_version);
    // Canonical display version (single source of truth across `--version`, `-V`, and `version`).
    let display_version =
        crate::app::version::format_display_version(name, version, app_git_sha_short);

    let mut root = clap::Command::new(name)
        .version(display_version)
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

    let mut tree = ClapTreeNode::default();
    for (path_str, cmd) in registry.all_tree_commands() {
        let segments: Vec<&str> = path_str.split('/').filter(|s| !s.is_empty()).collect();
        tree.insert_command(&segments, cmd);
    }
    for (path_str, _) in registry.groups() {
        let segments: Vec<&str> = path_str.split('/').filter(|s| !s.is_empty()).collect();
        tree.insert_group(&segments);
    }

    for (segment, node) in &tree.children {
        if let Some(sub) = build_clap_node(registry, segment, segment, node) {
            root = root.subcommand(sub);
        }
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

    root
}

fn build_clap_node(
    registry: &CommandRegistry,
    segment: &str,
    path_str: &str,
    node: &ClapTreeNode<'_>,
) -> Option<clap::Command> {
    if node.children.is_empty() {
        return node
            .command
            .map(|cmd| build_leaf_clap_command(segment, cmd));
    }

    let summary = registry
        .group_metadata_for(path_str)
        .map(|m| m.summary)
        .or_else(|| node.command.map(|cmd| cmd.summary))
        .unwrap_or("Command group");
    let static_name: &'static str = Box::leak(segment.to_string().into_boxed_str());
    let mut group = clap::Command::new(static_name)
        .about(summary)
        .subcommand_required(true)
        .arg_required_else_help(true);

    for (child_segment, child_node) in &node.children {
        let child_path = format!("{}/{}", path_str, child_segment);
        if let Some(child) = build_clap_node(registry, child_segment, &child_path, child_node) {
            group = group.subcommand(child);
        }
    }

    Some(group)
}

fn build_leaf_clap_command(segment: &str, cmd: &crate::command::Command) -> clap::Command {
    let static_name: &'static str = Box::leak(segment.to_string().into_boxed_str());
    let mut sub = if let Some(ref spec) = cmd.spec {
        build_typed_clap_command(static_name, spec)
    } else {
        build_legacy_clap_command_with_name(static_name, cmd)
    };

    if let Some(ref spec) = cmd.spec {
        for alias in &spec.aliases {
            sub = sub.visible_alias(alias);
        }
    }

    sub
}

fn build_legacy_clap_command_with_name(
    name: &'static str,
    cmd: &crate::command::Command,
) -> clap::Command {
    log::warn!(
        "legacy-parse-path: command '{}' has no ArgSpec; using trailing var-arg",
        cmd.id
    );

    let mut sub = clap::Command::new(name).about(cmd.summary);

    #[cfg(not(feature = "strict-args"))]
    {
        sub = sub.arg(
            clap::Arg::new("trailing")
                .num_args(0..)
                .trailing_var_arg(true)
                .allow_hyphen_values(true),
        );
    }

    if let Some(syntax) = cmd.syntax {
        sub = sub.after_help(format!("Syntax: {}", syntax));
    }

    sub
}

/// Recursively walk `ArgMatches` to extract the full `CommandPath` and the leaf `ArgMatches`.
///
/// Example: for `prog mcp serve --port 9090`, the root matches has "mcp" as subcommand,
/// whose matches has "serve" as subcommand. Returns `CommandPath(["mcp", "serve"])` and
/// the serve-level ArgMatches (which contains `--port`).
pub fn extract_nested_command_path(matches: &clap::ArgMatches) -> (CommandPath, &clap::ArgMatches) {
    let mut segments = Vec::new();
    let leaf = walk_subcommands(matches, &mut segments);
    (CommandPath(segments), leaf)
}

fn walk_subcommands<'a>(
    matches: &'a clap::ArgMatches,
    segments: &mut Vec<String>,
) -> &'a clap::ArgMatches {
    match matches.subcommand() {
        Some((name, sub)) => {
            segments.push(name.to_string());
            walk_subcommands(sub, segments)
        }
        None => matches,
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
            let (command_path, leaf_matches) = extract_nested_command_path(&matches);
            let leaf_name = command_path.leaf().unwrap_or("");

            // Look up by full path for multi-segment, flat map for single-segment.
            let cmd = if command_path.0.len() > 1 {
                registry.resolve(&command_path)
            } else {
                registry.get(leaf_name)
            };

            let (cmd_args, typed_args) = if let Some(cmd) = cmd {
                if let Some(ref spec) = cmd.spec {
                    match map_matches_to_typed_args(spec, leaf_matches) {
                        Ok(typed) => (CommandArgs::default(), Some(typed)),
                        Err(d) => return ParseOutcome::ParseError(d),
                    }
                } else {
                    (match_to_command_args(leaf_matches), None)
                }
            } else {
                // Built-in commands (e.g. "version") not in the user registry
                (match_to_command_args(leaf_matches), None)
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
                    // Prefer the framework's canonical version string (as set on the clap root)
                    // over clap's default "{display_name} {ver}" rendering, to keep output
                    // consistent with `App::version_string()`.
                    let text = root.get_version().unwrap_or_default().to_string();
                    ParseOutcome::VersionShown(text)
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
                    // Heuristic: if there are more than 2 argv tokens (prog + group + sub),
                    // this looks like a failed nested command lookup → E012.
                    let is_nested_attempt = args.len() > 2
                        && args.get(1).map(|a| !a.starts_with('-')).unwrap_or(false)
                        && args.get(2).map(|a| !a.starts_with('-')).unwrap_or(false);
                    if is_nested_attempt {
                        let path_str = args[1..args.len().min(args.len())]
                            .iter()
                            .take_while(|a| !a.starts_with('-'))
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(" ");
                        ParseOutcome::ParseError(Diagnostic {
                            code: E_NESTED_COMMAND_NOT_FOUND,
                            category: DiagnosticCategory::Parse,
                            message: format!("nested command path '{}' not found", path_str),
                            suggestion: Some("Use --help to see available commands".to_string()),
                            span: Some(cmd_arg),
                        })
                    } else {
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
}

fn match_to_command_args(sub_matches: &clap::ArgMatches) -> CommandArgs {
    let mut positional = Vec::new();
    let mut named = std::collections::HashMap::new();

    if let Some(values) = sub_matches.get_many::<String>("trailing") {
        let args: Vec<&str> = values.map(|s| s.as_str()).collect();
        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if let Some(stripped) = arg.strip_prefix("--") {
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
