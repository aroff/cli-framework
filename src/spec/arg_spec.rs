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
    /// Minimum number of occurrences for a [`Cardinality::Repeated`] arg.
    ///
    /// `Cardinality::Repeated` on its own says nothing about whether a value is
    /// mandatory — `--header` (zero or more) and a `<skill-ids>...` positional
    /// (one or more) are both `Repeated`. This field is how a spec says which:
    /// `Some(1)` (or more) makes the arg mandatory, so it is listed in the
    /// generated JSON Schema `required` array and carries `minItems` /
    /// `minimum`. `None` and `Some(0)` both mean zero-or-more, which is the
    /// pre-existing behavior and the `Default`.
    ///
    /// Ignored for `Required` (already mandatory) and `Optional` cardinalities.
    /// Schema-level only: CLI parsing arity is unchanged.
    pub min_occurs: Option<usize>,
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
    /// Whether the generated JSON Schema must list this arg in `required`.
    ///
    /// True for [`Cardinality::Required`], and for a [`Cardinality::Repeated`]
    /// arg that declares a minimum arity of at least one via
    /// [`ArgSpec::min_occurs`] — a one-or-more argument is mandatory, and an
    /// agent reading the schema has no other way to tell it apart from a
    /// zero-or-more one.
    pub fn is_schema_required(&self) -> bool {
        match self.cardinality {
            Cardinality::Required => true,
            Cardinality::Repeated => self.min_occurs.is_some_and(|min| min >= 1),
            Cardinality::Optional => false,
        }
    }

    /// The declared minimum arity, or `None` when the arg is zero-or-more.
    fn positive_min_occurs(&self) -> Option<usize> {
        self.min_occurs.filter(|min| *min >= 1)
    }

    /// Returns (property_name, schema_value).
    pub fn to_json_schema_property(&self) -> (String, Value) {
        let prop_name = self.long.unwrap_or(self.name).to_string();

        // NOTE: every branch below falls through to the shared `description`
        // insertion at the end of this function. Do not `return` early from
        // one — the repeated shapes used to, and that is exactly how
        // `ArgSpec.help` went missing from array/count properties.
        let mut schema_value = if self.cardinality == Cardinality::Repeated {
            let mut obj = serde_json::Map::new();
            if self.kind == ArgKind::Flag {
                // Repeated flag: an occurrence count (`-vvv` -> 3).
                obj.insert("type".to_string(), json!("integer"));
                if let Some(min) = self.positive_min_occurs() {
                    obj.insert("minimum".to_string(), json!(min));
                }
            } else {
                obj.insert("type".to_string(), json!("array"));
                obj.insert("items".to_string(), json!({ "type": "string" }));
                if let Some(min) = self.positive_min_occurs() {
                    obj.insert("minItems".to_string(), json!(min));
                }
            }
            Value::Object(obj)
        } else {
            self.scalar_schema()
        };
        if !self.help.is_empty() {
            if let Some(obj) = schema_value.as_object_mut() {
                obj.insert("description".to_string(), json!(self.help));
            }
        }
        (prop_name, schema_value)
    }

    /// The schema for a non-repeated arg, by declared value type.
    fn scalar_schema(&self) -> Value {
        match &self.value_type {
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
        }
    }
}
