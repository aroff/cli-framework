use crate::parser::diagnostic::Diagnostic;
use crate::spec::command_tree::CommandPath;
use crate::spec::value::ArgValue;
use std::collections::HashMap;

/// The result of a single parse attempt.
#[derive(Debug)]
pub enum ParseOutcome {
    /// A command was successfully parsed and all args typed.
    Parsed {
        command_path: CommandPath,
        /// Typed argument map (ArgValue per named arg). Empty for built-in commands
        /// that carry no args (e.g. "version").
        args: HashMap<String, ArgValue>,
        /// Typed global argument map (ArgValue per named global flag). Empty when no
        /// global flags are registered or none were provided.
        global_args: HashMap<String, ArgValue>,
    },
    HelpShown(String),
    VersionShown(String),
    ParseError(Diagnostic),
}
