use crate::spec::value::ArgValue;

/// Declares a single argument for a command.
#[derive(Debug, Clone)]
pub struct ArgSpec {
    pub name: &'static str,
    pub kind: ArgKind,
    pub short: Option<char>,
    /// Overrides the long flag name used in CLI and MCP schema. Falls back to `name` if None.
    pub long: Option<&'static str>,
    pub value_type: ArgValueType,
    pub cardinality: Cardinality,
    pub default: Option<ArgValue>,
    pub conflicts_with: Vec<&'static str>,
    pub requires: Vec<&'static str>,
    pub help: &'static str,
}

/// Argument kind.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgKind {
    /// Boolean presence; no value token (`--verbose`).
    Flag,
    /// Key-value option (`--output json`).
    Option,
    /// Positional argument.
    Positional,
}

/// The value type expected for an argument.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgValueType {
    Bool,
    String,
    Int,
    Float,
    /// Exhaustive set of allowed string tokens.
    Enum(Vec<&'static str>),
}

/// Cardinality of an argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Cardinality {
    /// Must appear exactly once.
    Required,
    /// May appear zero or one time.
    Optional,
    /// May appear one or more times; value becomes ArgValue::List.
    Repeated,
}
