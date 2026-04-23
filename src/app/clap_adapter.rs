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
    let mut root = clap::Command::new(app_name)
        .version(app_version)
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
                if let Some(eq_pos) = stripped.find('=') {
                    let key = &stripped[..eq_pos];
                    let value = &stripped[eq_pos + 1..];
                    named.insert(key.to_string(), value.to_string());
                    i += 1;
                } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                    named.insert(stripped.to_string(), args[i + 1].to_string());
                    i += 2;
                } else {
                    named.insert(stripped.to_string(), "true".to_string());
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
