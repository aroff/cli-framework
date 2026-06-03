//! Converts `CommandSpec` / `ArgSpec` into `clap::Command` / `clap::Arg` instances,
//! and maps `clap::ArgMatches` back to the typed `ArgValue` map.

use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::E_INVALID_VALUE;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::CommandSpec;
use crate::spec::value::ArgValue;
use std::collections::HashMap;

/// Build a fully-typed `clap::Command` from a `CommandSpec`.
pub fn build_typed_clap_command(id: &str, spec: &CommandSpec) -> clap::Command {
    let mut cmd = clap::Command::new(id.to_owned()).about(spec.summary);

    if let Some(long_about) = spec.long_about {
        cmd = cmd.long_about(long_about);
    }

    if let Some(syntax) = spec.syntax {
        cmd = cmd.after_help(format!("Syntax: {}", syntax));
    }

    for arg_spec in &spec.args {
        cmd = cmd.arg(build_clap_arg(arg_spec));
    }

    cmd
}

/// Convert `ArgMatches` to a typed arg map. Returns `Err(Diagnostic{E004})` on type mismatch.
pub fn map_matches_to_typed_args(
    spec: &CommandSpec,
    matches: &clap::ArgMatches,
) -> Result<HashMap<String, ArgValue>, Diagnostic> {
    let mut result = HashMap::new();

    fn is_user_provided(matches: &clap::ArgMatches, name: &str) -> bool {
        matches
            .value_source(name)
            .is_some_and(|s| s == clap::parser::ValueSource::CommandLine)
    }

    for arg_spec in &spec.args {
        match arg_spec.kind {
            ArgKind::Flag => match arg_spec.cardinality {
                Cardinality::Repeated => {
                    if is_user_provided(matches, arg_spec.name) {
                        let count = matches.get_count(arg_spec.name);
                        if count > 0 {
                            result.insert(arg_spec.name.to_string(), ArgValue::Count(count.into()));
                        }
                    }
                }
                _ => {
                    if is_user_provided(matches, arg_spec.name) {
                        let val = matches.get_flag(arg_spec.name);
                        if val {
                            result.insert(arg_spec.name.to_string(), ArgValue::Bool(true));
                        }
                    }
                }
            },
            ArgKind::Option | ArgKind::Positional => match arg_spec.cardinality {
                Cardinality::Repeated => {
                    if is_user_provided(matches, arg_spec.name) {
                        if let Some(vals) = matches.get_many::<String>(arg_spec.name) {
                            let list: Vec<ArgValue> = vals
                                .map(|v| coerce_value(v, &arg_spec.value_type, arg_spec.name))
                                .collect::<Result<Vec<_>, _>>()?;
                            result.insert(arg_spec.name.to_string(), ArgValue::List(list));
                        }
                    }
                }
                _ => {
                    if is_user_provided(matches, arg_spec.name) {
                        if let Some(val) = matches.get_one::<String>(arg_spec.name) {
                            let typed = coerce_value(val, &arg_spec.value_type, arg_spec.name)?;
                            result.insert(arg_spec.name.to_string(), typed);
                        }
                    }
                }
            },
        }
    }

    // Inject spec-declared defaults for any arg not provided by the user.
    // This makes `ArgSpec.default` authoritative at runtime, not just for `--help`,
    // so execute closures receive the default in `args.named` without re-declaring it.
    for arg_spec in &spec.args {
        if !result.contains_key(arg_spec.name) {
            if let Some(ref default) = arg_spec.default {
                result.insert(arg_spec.name.to_string(), default.clone());
            }
        }
    }

    Ok(result)
}

fn build_clap_arg(arg_spec: &ArgSpec) -> clap::Arg {
    let mut arg = clap::Arg::new(arg_spec.name).help(arg_spec.help);

    if let Some(short) = arg_spec.short {
        arg = arg.short(short);
    }

    if arg_spec.cardinality != Cardinality::Required {
        if let Some(ref default) = arg_spec.default {
            if !matches!(default, crate::spec::value::ArgValue::List(_)) {
                let default_str = default.to_string();
                if !default_str.is_empty() {
                    arg = arg.default_value(default_str);
                }
            }
        }
    }

    match arg_spec.kind {
        ArgKind::Flag => match arg_spec.cardinality {
            Cardinality::Repeated => {
                arg = arg
                    .long(arg_spec.long.unwrap_or(arg_spec.name))
                    .action(clap::ArgAction::Count);
            }
            _ => {
                arg = arg
                    .long(arg_spec.long.unwrap_or(arg_spec.name))
                    .action(clap::ArgAction::SetTrue);
                if arg_spec.cardinality == Cardinality::Required {
                    arg = arg.required(true);
                }
            }
        },
        ArgKind::Option => {
            arg = arg.long(arg_spec.long.unwrap_or(arg_spec.name));
            if let ArgValueType::Enum(allowed) = &arg_spec.value_type {
                // Add possible values to help text for discoverability.
                // Framework-level Enum validation is done in coerce_value (R4a).
                let enhanced = if arg_spec.help.is_empty() {
                    format!("[possible: {}]", allowed.join("|"))
                } else {
                    format!("{} [possible: {}]", arg_spec.help, allowed.join("|"))
                };
                arg = arg.help(enhanced);
            }
            match arg_spec.cardinality {
                Cardinality::Required => {
                    arg = arg.required(true);
                }
                Cardinality::Repeated => {
                    arg = arg.action(clap::ArgAction::Append);
                }
                Cardinality::Optional => {}
            }
        }
        ArgKind::Positional => match arg_spec.cardinality {
            Cardinality::Required => {
                arg = arg.required(true);
            }
            Cardinality::Repeated => {
                arg = arg.num_args(0..);
            }
            Cardinality::Optional => {}
        },
    }

    arg
}

fn coerce_value(s: &str, value_type: &ArgValueType, name: &str) -> Result<ArgValue, Diagnostic> {
    match value_type {
        ArgValueType::Bool => match s {
            "true" | "1" => Ok(ArgValue::Bool(true)),
            "false" | "0" => Ok(ArgValue::Bool(false)),
            _ => Err(type_error(name, s, "bool")),
        },
        ArgValueType::String => Ok(ArgValue::Str(s.to_string())),
        ArgValueType::Int => s
            .parse::<i64>()
            .map(ArgValue::Int)
            .map_err(|_| type_error(name, s, "integer")),
        ArgValueType::Float => s
            .parse::<f64>()
            .map(ArgValue::Float)
            .map_err(|_| type_error(name, s, "float")),
        ArgValueType::Enum(allowed) => {
            if allowed.contains(&s) {
                Ok(ArgValue::Enum(s.to_string()))
            } else {
                Err(Diagnostic {
                    code: E_INVALID_VALUE,
                    category: DiagnosticCategory::Spec,
                    message: format!(
                        "invalid value '{}' for '{}'; expected one of: {}",
                        s,
                        name,
                        allowed.join(", ")
                    ),
                    suggestion: Some(format!("Allowed values: {}", allowed.join(", "))),
                    span: Some(s.to_string()),
                })
            }
        }
    }
}

fn type_error(name: &str, value: &str, expected: &str) -> Diagnostic {
    Diagnostic {
        code: E_INVALID_VALUE,
        category: DiagnosticCategory::Spec,
        message: format!(
            "invalid value '{}' for '{}'; expected {}",
            value, name, expected
        ),
        suggestion: Some(format!("Provide a valid {} value for --{}", expected, name)),
        span: Some(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
    use crate::spec::command_tree::CommandSpec;

    fn make_spec(args: Vec<ArgSpec>) -> CommandSpec {
        CommandSpec {
            summary: "test",
            args,
            ..Default::default()
        }
    }

    #[test]
    fn build_typed_clap_command_contains_named_args() {
        let spec = make_spec(vec![
            ArgSpec {
                name: "output",
                kind: ArgKind::Option,
                short: None,
                long: None,
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Output format",
                ..Default::default()
            },
            ArgSpec {
                name: "verbose",
                kind: ArgKind::Flag,
                short: Some('v'),
                long: None,
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Verbose output",
                ..Default::default()
            },
        ]);

        let cmd = build_typed_clap_command("test", &spec);
        let arg_ids: Vec<_> = cmd.get_arguments().map(|a| a.get_id().as_str()).collect();
        assert!(arg_ids.contains(&"output"), "expected 'output' arg");
        assert!(arg_ids.contains(&"verbose"), "expected 'verbose' arg");
    }

    #[test]
    fn map_matches_typed_string() {
        let spec = make_spec(vec![ArgSpec {
            name: "name",
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Optional,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "",
            ..Default::default()
        }]);

        let cmd = build_typed_clap_command("test", &spec);
        let matches = cmd
            .try_get_matches_from(["test", "--name", "Alice"])
            .unwrap();
        let typed = map_matches_to_typed_args(&spec, &matches).unwrap();
        assert_eq!(typed.get("name"), Some(&ArgValue::Str("Alice".to_string())));
    }

    #[test]
    fn map_matches_flag_sets_bool_true() {
        let spec = make_spec(vec![ArgSpec {
            name: "verbose",
            kind: ArgKind::Flag,
            short: None,
            long: None,
            value_type: ArgValueType::Bool,
            cardinality: Cardinality::Optional,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "",
            ..Default::default()
        }]);

        let cmd = build_typed_clap_command("test", &spec);
        let matches = cmd.try_get_matches_from(["test", "--verbose"]).unwrap();
        let typed = map_matches_to_typed_args(&spec, &matches).unwrap();
        assert_eq!(typed.get("verbose"), Some(&ArgValue::Bool(true)));
    }

    #[test]
    fn map_matches_enum_valid() {
        let spec = make_spec(vec![ArgSpec {
            name: "format",
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::Enum(vec!["json", "text"]),
            cardinality: Cardinality::Optional,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "",
            ..Default::default()
        }]);

        let cmd = build_typed_clap_command("test", &spec);
        let matches = cmd
            .try_get_matches_from(["test", "--format", "json"])
            .unwrap();
        let typed = map_matches_to_typed_args(&spec, &matches).unwrap();
        assert_eq!(
            typed.get("format"),
            Some(&ArgValue::Enum("json".to_string()))
        );
    }

    #[test]
    fn map_matches_list_accumulation() {
        let spec = make_spec(vec![ArgSpec {
            name: "tag",
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Repeated,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "",
            ..Default::default()
        }]);

        let cmd = build_typed_clap_command("test", &spec);
        let matches = cmd
            .try_get_matches_from(["test", "--tag", "a", "--tag", "b"])
            .unwrap();
        let typed = map_matches_to_typed_args(&spec, &matches).unwrap();
        assert_eq!(
            typed.get("tag"),
            Some(&ArgValue::List(vec![
                ArgValue::Str("a".to_string()),
                ArgValue::Str("b".to_string()),
            ]))
        );
    }
}
