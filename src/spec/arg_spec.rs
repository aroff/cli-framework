use crate::spec::value::ArgValue;
use serde_json::{json, Value};

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

impl ArgSpec {
    /// Returns (property_name, schema_value).
    pub fn to_json_schema_property(&self) -> (String, Value) {
        let prop_name = self.long.unwrap_or(self.name).to_string();

        if self.cardinality == Cardinality::Repeated {
            let schema_value = if self.kind == ArgKind::Flag {
                json!({ "type": "integer" })
            } else {
                json!({ "type": "array", "items": { "type": "string" } })
            };
            return (prop_name, schema_value);
        }

        let schema_value = match &self.value_type {
            ArgValueType::Bool => json!({ "type": "boolean" }),
            ArgValueType::String => json!({ "type": "string" }),
            ArgValueType::Int => json!({ "type": "integer" }),
            ArgValueType::Float => json!({ "type": "number" }),
            ArgValueType::Enum(variants) => json!({
                "type": "string",
                "enum": variants,
            }),
        };
        (prop_name, schema_value)
    }
}
