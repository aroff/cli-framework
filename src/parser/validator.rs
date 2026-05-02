use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::{
    E_CONFLICT, E_INVALID_VALUE, E_MISSING_REQUIRED, E_UNSATISFIED_REQUIRES,
};
use crate::spec::arg_spec::{ArgValueType, Cardinality};
use crate::spec::command_tree::CommandSpec;
use crate::spec::value::ArgValue;
use std::collections::HashMap;

/// Stateless spec-constraint validator (Stage 2 of the validation pipeline).
pub struct SpecValidator;

impl SpecValidator {
    /// Validate typed args against the spec. Returns all violations found.
    pub fn validate(spec: &CommandSpec, args: &HashMap<String, ArgValue>) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Required-arg check (E003)
        for arg_spec in &spec.args {
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
        for arg_spec in &spec.args {
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
        for arg_spec in &spec.args {
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
        for arg_spec in &spec.args {
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
        ArgValue::Enum(v) => {
            if let ArgValueType::Enum(allowed) = value_type {
                allowed.contains(&v.as_str())
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::arg_spec::{ArgKind, ArgSpec};
    use crate::spec::command_tree::CommandSpec;

    fn make_spec_with_args(args: Vec<ArgSpec>) -> CommandSpec {
        CommandSpec {
            args,
            ..Default::default()
        }
    }

    fn required_str_arg(name: &'static str) -> ArgSpec {
        ArgSpec {
            name,
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Required,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "",
        }
    }

    fn optional_str_arg(name: &'static str) -> ArgSpec {
        ArgSpec {
            name,
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Optional,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "",
        }
    }

    // E003: missing required
    #[test]
    fn e003_missing_required_arg() {
        let spec = make_spec_with_args(vec![required_str_arg("output")]);
        let args = HashMap::new();
        let diags = SpecValidator::validate(&spec, &args);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "E003");
        assert!(diags[0].suggestion.as_deref().unwrap().contains("output"));
    }

    #[test]
    fn e003_required_arg_present_no_error() {
        let spec = make_spec_with_args(vec![required_str_arg("output")]);
        let mut args = HashMap::new();
        args.insert("output".to_string(), ArgValue::Str("json".to_string()));
        let diags = SpecValidator::validate(&spec, &args);
        assert!(diags.iter().all(|d| d.code != "E003"));
    }

    // E004: type mismatch
    #[test]
    fn e004_wrong_type() {
        let spec = make_spec_with_args(vec![ArgSpec {
            name: "count",
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::Int,
            cardinality: Cardinality::Optional,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "",
        }]);
        let mut args = HashMap::new();
        args.insert("count".to_string(), ArgValue::Str("notanumber".to_string()));
        let diags = SpecValidator::validate(&spec, &args);
        assert!(diags.iter().any(|d| d.code == "E004"));
    }

    #[test]
    fn e004_correct_type_no_error() {
        let spec = make_spec_with_args(vec![ArgSpec {
            name: "count",
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::Int,
            cardinality: Cardinality::Optional,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "",
        }]);
        let mut args = HashMap::new();
        args.insert("count".to_string(), ArgValue::Int(42));
        let diags = SpecValidator::validate(&spec, &args);
        assert!(diags.iter().all(|d| d.code != "E004"));
    }

    // E005: conflict
    #[test]
    fn e005_conflict_both_present() {
        let spec = make_spec_with_args(vec![
            ArgSpec {
                name: "arg_a",
                kind: ArgKind::Flag,
                short: None,
                long: None,
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec!["arg_b"],
                requires: vec![],
                help: "",
            },
            optional_str_arg("arg_b"),
        ]);
        let mut args = HashMap::new();
        args.insert("arg_a".to_string(), ArgValue::Bool(true));
        args.insert("arg_b".to_string(), ArgValue::Str("x".to_string()));
        let diags = SpecValidator::validate(&spec, &args);
        let conflict_diags: Vec<_> = diags.iter().filter(|d| d.code == "E005").collect();
        assert_eq!(conflict_diags.len(), 1);
        assert!(conflict_diags[0].suggestion.is_some());
    }

    #[test]
    fn e005_no_conflict_when_only_one_present() {
        let spec = make_spec_with_args(vec![
            ArgSpec {
                name: "arg_a",
                kind: ArgKind::Flag,
                short: None,
                long: None,
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec!["arg_b"],
                requires: vec![],
                help: "",
            },
            optional_str_arg("arg_b"),
        ]);
        let mut args = HashMap::new();
        args.insert("arg_a".to_string(), ArgValue::Bool(true));
        let diags = SpecValidator::validate(&spec, &args);
        assert!(diags.iter().all(|d| d.code != "E005"));
    }

    // E006: unsatisfied requires
    #[test]
    fn e006_requires_missing() {
        let spec = make_spec_with_args(vec![
            ArgSpec {
                name: "arg_a",
                kind: ArgKind::Flag,
                short: None,
                long: None,
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec!["arg_b"],
                help: "",
            },
            optional_str_arg("arg_b"),
        ]);
        let mut args = HashMap::new();
        args.insert("arg_a".to_string(), ArgValue::Bool(true));
        let diags = SpecValidator::validate(&spec, &args);
        let req_diags: Vec<_> = diags.iter().filter(|d| d.code == "E006").collect();
        assert_eq!(req_diags.len(), 1);
        assert!(req_diags[0].suggestion.is_some());
    }

    #[test]
    fn e006_requires_satisfied() {
        let spec = make_spec_with_args(vec![
            ArgSpec {
                name: "arg_a",
                kind: ArgKind::Flag,
                short: None,
                long: None,
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec!["arg_b"],
                help: "",
            },
            optional_str_arg("arg_b"),
        ]);
        let mut args = HashMap::new();
        args.insert("arg_a".to_string(), ArgValue::Bool(true));
        args.insert("arg_b".to_string(), ArgValue::Str("val".to_string()));
        let diags = SpecValidator::validate(&spec, &args);
        assert!(diags.iter().all(|d| d.code != "E006"));
    }

    #[test]
    fn all_suggestions_non_empty_in_validate_errors() {
        let spec = make_spec_with_args(vec![required_str_arg("out")]);
        let args = HashMap::new();
        let diags = SpecValidator::validate(&spec, &args);
        for d in &diags {
            assert!(
                d.suggestion
                    .as_deref()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false),
                "suggestion must be non-empty for code {}",
                d.code
            );
        }
    }
}
