use crate::spec::arg_spec::ArgSpec;
use crate::spec::arg_spec::{ArgValueType, Cardinality};
use crate::spec::value::ArgValue;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// Command-level metadata for typed spec-driven commands.
#[derive(Debug, Clone, Default)]
pub struct CommandSpec {
    pub summary: &'static str,
    pub long_about: Option<&'static str>,
    pub examples: Vec<&'static str>,
    pub aliases: Vec<&'static str>,
    pub hidden_aliases: Vec<&'static str>,
    pub hidden: bool,
    /// Deprecation message, if this command is deprecated.
    pub deprecated: Option<&'static str>,
    pub env_vars: Vec<EnvVarEntry>,
    pub exit_codes: Vec<ExitCodeEntry>,
    pub args: Vec<ArgSpec>,
    pub notes: Option<&'static str>,
}

impl CommandSpec {
    /// Builds the full JSON Schema object for this spec's args.
    pub fn to_json_schema(&self) -> Value {
        let mut properties: BTreeMap<String, Value> = BTreeMap::new();
        let mut required: Vec<String> = Vec::new();

        for arg in &self.args {
            let (prop_name, schema_value) = arg.to_json_schema_property();
            properties.insert(prop_name.clone(), schema_value);
            if arg.cardinality == Cardinality::Required {
                required.push(prop_name);
            }
        }

        let props_value: Value = serde_json::to_value(&properties).unwrap_or(json!({}));

        let mut schema = json!({
            "type": "object",
            "properties": props_value,
        });

        if !required.is_empty() {
            schema["required"] = Value::Array(required.into_iter().map(Value::String).collect());
        }

        schema
    }

    /// Validates typed args against spec constraints.
    ///
    /// MUST preserve the same diagnostic semantics as SpecValidator::validate today.
    pub fn validate_typed_args(
        &self,
        args: &HashMap<String, ArgValue>,
    ) -> Vec<crate::parser::diagnostic::Diagnostic> {
        use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
        use crate::parser::error_codes::{
            E_CONFLICT, E_INVALID_VALUE, E_MISSING_REQUIRED, E_UNSATISFIED_REQUIRES,
        };

        let mut diagnostics = Vec::new();

        // Required-arg check (E003)
        for arg_spec in &self.args {
            if arg_spec.cardinality == Cardinality::Required && !args.contains_key(arg_spec.name) {
                diagnostics.push(Diagnostic {
                    code: E_MISSING_REQUIRED,
                    category: DiagnosticCategory::Spec,
                    message: format!("missing required argument '--{}'", arg_spec.name),
                    suggestion: Some(format!("Provide --{} <value>", arg_spec.name)),
                    span: None,
                });
            }
        }

        // Type-check (E004) — verifies ArgValue variant matches declared ArgValueType
        for arg_spec in &self.args {
            if let Some(value) = args.get(arg_spec.name) {
                if !value_matches_type(value, &arg_spec.value_type) {
                    diagnostics.push(Diagnostic {
                        code: E_INVALID_VALUE,
                        category: DiagnosticCategory::Spec,
                        message: format!(
                            "invalid value type for '--{}': expected {:?}",
                            arg_spec.name, arg_spec.value_type
                        ),
                        suggestion: Some(format!("Provide a valid value for --{}", arg_spec.name)),
                        span: None,
                    });
                }
            }
        }

        // Conflict check (E005)
        for arg_spec in &self.args {
            if args.contains_key(arg_spec.name) {
                for conflicting in &arg_spec.conflicts_with {
                    if args.contains_key(*conflicting) {
                        diagnostics.push(Diagnostic {
                            code: E_CONFLICT,
                            category: DiagnosticCategory::Spec,
                            message: format!(
                                "--{} conflicts with --{}",
                                arg_spec.name, conflicting
                            ),
                            suggestion: Some(format!(
                                "Remove --{} or --{}",
                                arg_spec.name, conflicting
                            )),
                            span: None,
                        });
                    }
                }
            }
        }

        // Requires check (E006)
        for arg_spec in &self.args {
            if args.contains_key(arg_spec.name) {
                for required_dep in &arg_spec.requires {
                    if !args.contains_key(*required_dep) {
                        diagnostics.push(Diagnostic {
                            code: E_UNSATISFIED_REQUIRES,
                            category: DiagnosticCategory::Spec,
                            message: format!("--{} requires --{}", arg_spec.name, required_dep),
                            suggestion: Some(format!("Also provide --{}", required_dep)),
                            span: None,
                        });
                    }
                }
            }
        }

        diagnostics
    }
}

fn value_matches_type(value: &ArgValue, value_type: &ArgValueType) -> bool {
    match value {
        // Count values (repeated flags) skip type enforcement
        ArgValue::Count(_) => true,
        ArgValue::List(vs) => vs.iter().all(|v| value_matches_type(v, value_type)),
        ArgValue::Bool(_) => matches!(value_type, ArgValueType::Bool),
        ArgValue::Str(_) => matches!(value_type, ArgValueType::String),
        ArgValue::Int(_) => matches!(value_type, ArgValueType::Int),
        ArgValue::Float(_) => matches!(value_type, ArgValueType::Float),
        ArgValue::Enum(_) => {
            // Only verify the value variant matches the declared Enum type; per-command
            // execute closures validate the specific allowed values with proper error codes.
            matches!(value_type, ArgValueType::Enum(_))
        }
    }
}

/// An environment variable referenced by a command.
#[derive(Debug, Clone)]
pub struct EnvVarEntry {
    pub name: &'static str,
    pub description: &'static str,
}

/// An exit code documented by a command.
#[derive(Debug, Clone)]
pub struct ExitCodeEntry {
    pub code: i32,
    pub description: &'static str,
}

/// Metadata for a command group (non-leaf path node).
#[derive(Debug, Clone, Default)]
pub struct GroupMetadata {
    pub summary: &'static str,
    pub hidden: bool,
}

/// Hierarchical command path (e.g. `["cluster", "get"]` → `"cluster/get"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CommandPath(pub Vec<String>);

impl CommandPath {
    /// Construct from string slices. Returns Err if any segment contains '/'.
    pub fn new(segments: &[&str]) -> Result<Self, CommandPathError> {
        for s in segments {
            if s.contains('/') {
                return Err(CommandPathError::InvalidSegment {
                    segment: s.to_string(),
                });
            }
        }
        Ok(CommandPath(
            segments.iter().map(|s| s.to_string()).collect(),
        ))
    }

    /// Convenience for a single root-level ID.
    pub fn root_for(id: &str) -> Self {
        CommandPath(vec![id.to_string()])
    }

    /// Returns `"a/b/c"` for path `["a", "b", "c"]`.
    pub fn to_path_string(&self) -> String {
        self.0.join("/")
    }

    /// Returns `None` for a root-level (single-segment) path.
    pub fn parent(&self) -> Option<CommandPath> {
        if self.0.len() <= 1 {
            None
        } else {
            Some(CommandPath(self.0[..self.0.len() - 1].to_vec()))
        }
    }

    /// Returns a new path with the given segment appended.
    pub fn push(&self, segment: &str) -> Result<CommandPath, CommandPathError> {
        if segment.contains('/') {
            return Err(CommandPathError::InvalidSegment {
                segment: segment.to_string(),
            });
        }
        let mut new_path = self.0.clone();
        new_path.push(segment.to_string());
        Ok(CommandPath(new_path))
    }

    /// Returns the final segment (leaf command name).
    pub fn leaf(&self) -> Option<&str> {
        self.0.last().map(|s| s.as_str())
    }
}

/// Error constructing a CommandPath.
#[derive(Debug, thiserror::Error)]
pub enum CommandPathError {
    #[error("path segment '{segment}' must not contain '/'")]
    InvalidSegment { segment: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_to_path_string_roundtrip() {
        let path = CommandPath::new(&["cluster", "get"]).unwrap();
        assert_eq!(path.to_path_string(), "cluster/get");
    }

    #[test]
    fn path_single_segment_to_string() {
        let path = CommandPath::root_for("hello");
        assert_eq!(path.to_path_string(), "hello");
    }

    #[test]
    fn path_parent_root_is_none() {
        let path = CommandPath::root_for("hello");
        assert!(path.parent().is_none());
    }

    #[test]
    fn path_parent_nested() {
        let path = CommandPath::new(&["cluster", "get"]).unwrap();
        assert_eq!(path.parent(), Some(CommandPath::root_for("cluster")));
    }

    #[test]
    fn path_push_success() {
        let path = CommandPath::root_for("cluster");
        let pushed = path.push("get").unwrap();
        assert_eq!(pushed.to_path_string(), "cluster/get");
    }

    #[test]
    fn path_push_slash_segment_error() {
        let path = CommandPath::root_for("cluster");
        let err = path.push("bad/segment").unwrap_err();
        match err {
            CommandPathError::InvalidSegment { segment } => {
                assert_eq!(segment, "bad/segment");
            }
        }
    }

    #[test]
    fn path_new_invalid_segment_error() {
        let err = CommandPath::new(&["bad/segment"]).unwrap_err();
        match err {
            CommandPathError::InvalidSegment { segment } => {
                assert_eq!(segment, "bad/segment");
            }
        }
    }

    #[test]
    fn path_leaf() {
        let path = CommandPath::new(&["cluster", "get"]).unwrap();
        assert_eq!(path.leaf(), Some("get"));
    }

    #[test]
    fn path_leaf_root() {
        let path = CommandPath::root_for("hello");
        assert_eq!(path.leaf(), Some("hello"));
    }

    #[test]
    fn path_empty_leaf_is_none() {
        let path = CommandPath(vec![]);
        assert!(path.leaf().is_none());
    }
}
