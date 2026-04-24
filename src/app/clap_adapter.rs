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
        let mut sub = clap::Command::new(cmd.id).about(cmd.summary).arg(
            clap::Arg::new("trailing")
                .num_args(0..)
                .trailing_var_arg(true)
                .allow_hyphen_values(true),
        );

        if let Some(syntax) = cmd.syntax {
            sub = sub.after_help(format!("Syntax: {}", syntax));
        }
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

    root
}

pub fn parse_with_clap(
    root: &clap::Command,
    args: Vec<String>,
) -> anyhow::Result<Option<ParsedCommand>> {
    match root.clone().try_get_matches_from(&args) {
        Ok(matches) => {
            let (name, sub_matches) = matches.subcommand().expect("subcommand required");
            let args = match_to_command_args(sub_matches);
            Ok(Some(ParsedCommand {
                command_id: name.to_string(),
                args,
            }))
        }
        Err(e) => {
            if !e.use_stderr() {
                use std::io::Write;
                let mut stdout = std::io::stdout();
                write!(stdout, "{}", e).ok();
                stdout.flush().ok();
            } else {
                e.print().ok();
            }
            Ok(None)
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
