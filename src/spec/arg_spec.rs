use crate::spec::value::ArgValue;
use serde_json::{json, Value};

/// Declares a single argument for a command.
#[derive(Debug, Clone, Default)]
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
    /// Numeric lower bound for Int args (inclusive).
    pub min: Option<i64>,
    /// Numeric upper bound for Int args (inclusive).
    pub max: Option<i64>,
    /// Numeric lower bound for Float args (inclusive).
    pub min_f: Option<f64>,
    /// Numeric upper bound for Float args (inclusive).
    pub max_f: Option<f64>,
    /// Regex pattern constraint for String args.
    pub pattern: Option<&'static str>,
}

/// Argument kind.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ArgKind {
    /// Boolean presence; no value token (`--verbose`).
    #[default]
    Flag,
    /// Key-value option (`--output json`).
    Option,
    /// Positional argument.
    Positional,
}

/// The value type expected for an argument.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ArgValueType {
    /// Unconstrained string value.
    #[default]
    String,
    Bool,
    Int,
    Float,
    /// Exhaustive set of allowed string tokens.
    Enum(Vec<&'static str>),
}

/// Cardinality of an argument.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Cardinality {
    /// Must appear exactly once.
    Required,
    /// May appear zero or one time.
    #[default]
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
            ArgValueType::String => {
                let mut obj = serde_json::Map::new();
                obj.insert("type".to_string(), json!("string"));
                if let Some(pat) = self.pattern {
                    obj.insert("pattern".to_string(), json!(pat));
                }
                Value::Object(obj)
            }
            ArgValueType::Int => {
                let mut obj = serde_json::Map::new();
                obj.insert("type".to_string(), json!("integer"));
                if let Some(min) = self.min {
                    obj.insert("minimum".to_string(), json!(min));
                }
                if let Some(max) = self.max {
                    obj.insert("maximum".to_string(), json!(max));
                }
                Value::Object(obj)
            }
            ArgValueType::Float => {
                let mut obj = serde_json::Map::new();
                obj.insert("type".to_string(), json!("number"));
                if let Some(min_f) = self.min_f {
                    obj.insert("minimum".to_string(), json!(min_f));
                }
                if let Some(max_f) = self.max_f {
                    obj.insert("maximum".to_string(), json!(max_f));
                }
                Value::Object(obj)
            }
            ArgValueType::Enum(variants) => json!({
                "type": "string",
                "enum": variants,
            }),
        };
        (prop_name, schema_value)
    }
}
