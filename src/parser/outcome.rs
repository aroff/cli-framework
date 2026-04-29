use crate::command::CommandArgs;
use crate::parser::diagnostic::Diagnostic;
use crate::spec::command_tree::CommandPath;
use crate::spec::value::ArgValue;
use std::collections::HashMap;

/// The result of a single parse attempt.
#[derive(Debug)]
pub enum ParseOutcome {
    /// A command was successfully parsed.
    Parsed {
        command_path: CommandPath,
        args: CommandArgs,
        /// Populated only for typed (spec-bearing) commands.
        typed_args: Option<HashMap<String, ArgValue>>,
    },
    /// `--help` / `-h` was requested; help was rendered; no command to execute.
    /// The String contains the help text to be printed.
    HelpShown(String),
    /// `--version` / `-V` was requested; version was rendered; no command to execute.
    /// The String contains the version text to be printed.
    VersionShown(String),
    /// A parse error occurred; diagnostics carry the details.
    ParseError(Diagnostic),
}
