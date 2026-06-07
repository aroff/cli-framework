//! Clap-based argv parsing adapter.
//!
//! **Design deviation from spec Section 5.1:** `build_clap_root` accepts
//! `app_name` and `app_version` as separate parameters in addition to
//! `meta: Option<&AppMeta>`, because `App` stores these independently of
//! `AppMeta` (fields `app_name` / `app_version` on the `App` struct). When
//! `meta` is `None`, the name and version still need to be propagated to
//! Clap.

use crate::app::AppMeta;
use crate::command::CommandRegistry;
use crate::parser::clap_mapper::{build_typed_clap_command, map_matches_to_typed_args};
use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::{
    E_MISSING_REQUIRED, E_NESTED_COMMAND_NOT_FOUND, E_UNKNOWN_COMMAND, E_UNKNOWN_FLAG,
};
use crate::parser::outcome::ParseOutcome;
use crate::spec::command_tree::CommandPath;
use crate::spec::value::ArgValue;
use std::collections::HashMap;

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
    global_flags: &[crate::spec::arg_spec::ArgSpec],
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

    use crate::parser::clap_mapper::build_clap_arg;
    for flag_spec in global_flags {
        root = root.arg(build_clap_arg(flag_spec).global(true));
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
        .or_else(|| node.command.map(|cmd| cmd.summary()))
        .unwrap_or("Command group");
    let mut group = clap::Command::new(segment.to_string())
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
    let mut sub = build_typed_clap_command(segment, &cmd.spec);
    for alias in &cmd.spec.aliases {
        sub = sub.visible_alias(*alias);
    }
    for alias in &cmd.spec.hidden_aliases {
        sub = sub.alias(*alias);
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
fn extract_clap_suggestion(e: &clap::Error, kind: clap::error::ContextKind) -> Option<String> {
    use clap::error::ContextValue;
    e.context().find_map(|(k, v)| {
        if k == kind {
            match v {
                ContextValue::String(s) => Some(s.clone()),
                ContextValue::Strings(ss) => ss.first().cloned(),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn format_subcommand_suggestion(candidate: &str) -> String {
    format!("Did you mean '{}'?", candidate)
}

fn format_flag_suggestion(candidate: &str) -> String {
    if candidate.starts_with("--") {
        format!("Did you mean '{}'?", candidate)
    } else {
        format!("Did you mean '--{}'?", candidate)
    }
}

pub fn parse_with_clap(
    root: &clap::Command,
    registry: &CommandRegistry,
    args: Vec<String>,
    global_flags: &[crate::spec::arg_spec::ArgSpec],
    suggest_corrections: bool,
) -> ParseOutcome {
    match root.clone().try_get_matches_from(&args) {
        Ok(matches) => {
            let (command_path, leaf_matches) = extract_nested_command_path(&matches);
            let leaf_name = command_path.leaf().unwrap_or("");

            let cmd = if command_path.0.len() > 1 {
                registry.resolve(&command_path)
            } else {
                registry.get(leaf_name)
            };

            let args: HashMap<String, ArgValue> = if let Some(cmd) = cmd {
                match map_matches_to_typed_args(&cmd.spec, leaf_matches) {
                    Ok(typed) => typed,
                    Err(d) => return ParseOutcome::ParseError(d),
                }
            } else {
                // Built-in commands (e.g. "version") not in user registry — no args
                HashMap::new()
            };

            // Extract global args from leaf_matches (globals propagate via .global(true))
            let global_args = if global_flags.is_empty() {
                HashMap::new()
            } else {
                let global_spec = crate::spec::command_tree::CommandSpec {
                    args: global_flags.to_vec(),
                    ..Default::default()
                };
                map_matches_to_typed_args(&global_spec, leaf_matches).unwrap_or_default()
            };

            ParseOutcome::Parsed {
                command_path,
                args,
                global_args,
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
                    // Note: clap typically renders version output with a trailing newline.
                    // Preserve that shape so callers that print `ParseOutcome::VersionShown`
                    // verbatim still emit a single line.
                    let mut text = root.get_version().unwrap_or_default().to_string();
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                    ParseOutcome::VersionShown(text)
                }
                ErrorKind::UnknownArgument => {
                    use clap::error::ContextKind;
                    let (suggestion, span) = if suggest_corrections {
                        let span = extract_clap_suggestion(&e, ContextKind::InvalidArg);
                        let suggestion = extract_clap_suggestion(&e, ContextKind::SuggestedArg)
                            .map(|s| format_flag_suggestion(&s))
                            .unwrap_or_else(|| "Use --help to see available arguments".to_string());
                        (suggestion, span)
                    } else {
                        ("Use --help to see available arguments".to_string(), None)
                    };
                    ParseOutcome::ParseError(Diagnostic {
                        code: E_UNKNOWN_FLAG,
                        category: DiagnosticCategory::Parse,
                        message: "unknown argument".to_string(),
                        suggestion: Some(suggestion),
                        span,
                    })
                }
                ErrorKind::MissingRequiredArgument => {
                    // Extract missing argument names from clap's error context.
                    use clap::error::ContextKind;
                    let missing: Vec<String> = e
                        .context()
                        .filter_map(|(kind, val)| {
                            if kind == ContextKind::InvalidArg {
                                Some(val.to_string())
                            } else {
                                None
                            }
                        })
                        .collect();
                    let arg_desc = if missing.is_empty() {
                        "required argument".to_string()
                    } else {
                        missing.join(", ")
                    };
                    ParseOutcome::ParseError(Diagnostic {
                        code: E_MISSING_REQUIRED,
                        category: DiagnosticCategory::Parse,
                        message: format!("missing required argument {}", arg_desc),
                        suggestion: Some("Use --help to see required arguments".to_string()),
                        span: None,
                    })
                }
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
                        let suggestion = if suggest_corrections {
                            use clap::error::ContextKind;
                            extract_clap_suggestion(&e, ContextKind::SuggestedSubcommand)
                                .map(|s| format_subcommand_suggestion(&s))
                                .unwrap_or_else(|| {
                                    "Use --help to see available commands".to_string()
                                })
                        } else {
                            "Use --help to see available commands".to_string()
                        };
                        ParseOutcome::ParseError(Diagnostic {
                            code: E_NESTED_COMMAND_NOT_FOUND,
                            category: DiagnosticCategory::Parse,
                            message: format!("nested command path '{}' not found", path_str),
                            suggestion: Some(suggestion),
                            span: Some(cmd_arg),
                        })
                    } else {
                        let suggestion = if suggest_corrections {
                            use clap::error::ContextKind;
                            extract_clap_suggestion(&e, ContextKind::SuggestedSubcommand)
                                .map(|s| format_subcommand_suggestion(&s))
                                .unwrap_or_else(|| {
                                    "Use --help to see available commands".to_string()
                                })
                        } else {
                            "Use --help to see available commands".to_string()
                        };
                        ParseOutcome::ParseError(Diagnostic {
                            code: E_UNKNOWN_COMMAND,
                            category: DiagnosticCategory::Parse,
                            message: format!("unrecognized subcommand '{}'", cmd_arg),
                            suggestion: Some(suggestion),
                            span: Some(cmd_arg),
                        })
                    }
                }
            }
        }
    }
}
