//! Converts `CommandSpec` / `ArgSpec` into `clap::Command` / `clap::Arg` instances,
//! and maps `clap::ArgMatches` back to the typed `ArgValue` map.

use crate::command::Command;
use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
use crate::parser::error_codes::E_INVALID_VALUE;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::CommandSpec;
use crate::spec::value::ArgValue;
use std::collections::HashMap;

/// Build a fully-typed `clap::Command` from a `CommandSpec`.
pub fn build_typed_clap_command(id: &'static str, spec: &CommandSpec) -> clap::Command {
    let mut cmd = clap::Command::new(id).about(spec.summary);

    if let Some(long_about) = spec.long_about {
        cmd = cmd.long_about(long_about);
    }

    for arg_spec in &spec.args {
        cmd = cmd.arg(build_clap_arg(arg_spec));
    }

    cmd
}

/// Build a legacy `clap::Command` with a trailing var-arg (no spec required).
pub fn build_legacy_clap_command(cmd: &Command) -> clap::Command {
    log::warn!(
        "legacy-parse-path: command '{}' has no ArgSpec; using trailing var-arg",
        cmd.id
    );

    let mut sub = clap::Command::new(cmd.id).about(cmd.summary);

    // With strict-args feature, don't use trailing_var_arg (reject unknown flags)
    #[cfg(not(feature = "strict-args"))]
    {
        sub = sub.arg(
            clap::Arg::new("trailing")
                .num_args(0..)
                .trailing_var_arg(true)
                .allow_hyphen_values(true),
        );
    }

    if let Some(syntax) = cmd.syntax {
        sub = sub.after_help(format!("Syntax: {}", syntax));
    }

    sub
}

/// Convert `ArgMatches` to a typed arg map. Returns `Err(Diagnostic{E004})` on type mismatch.
pub fn map_matches_to_typed_args(
    spec: &CommandSpec,
    matches: &clap::ArgMatches,
) -> Result<HashMap<String, ArgValue>, Diagnostic> {
    let mut result = HashMap::new();

    for arg_spec in &spec.args {
        match arg_spec.kind {
            ArgKind::Flag => match arg_spec.cardinality {
                Cardinality::Repeated => {
                    let count = matches.get_count(arg_spec.name);
                    if count > 0 {
                        result.insert(arg_spec.name.to_string(), ArgValue::Count(count.into()));
                    }
                }
                _ => {
                    let val = matches.get_flag(arg_spec.name);
                    if val {
                        result.insert(arg_spec.name.to_string(), ArgValue::Bool(true));
                    }
                }
            },
            ArgKind::Option | ArgKind::Positional => match arg_spec.cardinality {
                Cardinality::Repeated => {
                    if let Some(vals) = matches.get_many::<String>(arg_spec.name) {
                        let list: Vec<ArgValue> = vals
                            .map(|v| coerce_value(v, &arg_spec.value_type, arg_spec.name))
                            .collect::<Result<Vec<_>, _>>()?;
                        result.insert(arg_spec.name.to_string(), ArgValue::List(list));
                    }
                }
                _ => {
                    if let Some(val) = matches.get_one::<String>(arg_spec.name) {
                        let typed = coerce_value(val, &arg_spec.value_type, arg_spec.name)?;
                        result.insert(arg_spec.name.to_string(), typed);
                    }
                }
            },
        }
    }

    Ok(result)
}

fn build_clap_arg(arg_spec: &ArgSpec) -> clap::Arg {
    let mut arg = clap::Arg::new(arg_spec.name).help(arg_spec.help);

    if let Some(short) = arg_spec.short {
        arg = arg.short(short);
    }

    // Apply default_value for optional/repeated args that declare a default.
    // Clap 4 requires `'static` strings; Box::leak is bounded by the number of
    // unique default values across all arg specs, which is negligible at startup.
    if arg_spec.cardinality != Cardinality::Required {
        if let Some(ref default) = arg_spec.default {
            use crate::spec::value::ArgValue;
            let default_str: &'static str = match default {
                ArgValue::Str(s) => Box::leak(s.clone().into_boxed_str()),
                ArgValue::Enum(s) => Box::leak(s.clone().into_boxed_str()),
                ArgValue::Bool(b) => {
                    if *b {
                        "true"
                    } else {
                        "false"
                    }
                }
                ArgValue::Int(i) => Box::leak(i.to_string().into_boxed_str()),
                ArgValue::Float(f) => Box::leak(f.to_string().into_boxed_str()),
                _ => "",
            };
            if !default_str.is_empty() {
                arg = arg.default_value(default_str);
            }
        }
    }

    match arg_spec.kind {
        ArgKind::Flag => match arg_spec.cardinality {
            Cardinality::Repeated => {
                arg = arg.long(arg_spec.name).action(clap::ArgAction::Count);
            }
            _ => {
                arg = arg.long(arg_spec.name).action(clap::ArgAction::SetTrue);
                if arg_spec.cardinality == Cardinality::Required {
                    arg = arg.required(true);
                }
            }
        },
        ArgKind::Option => {
            arg = arg.long(arg_spec.name);
            if let ArgValueType::Enum(allowed) = &arg_spec.value_type {
                // Add possible values to help text for discoverability without clap-level
                // validation, so per-command execute closures can return proper error codes.
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
        ArgValueType::Enum(_) => {
            // Wrap any string as an Enum value; per-command execute closures are
            // responsible for validating the specific allowed values and returning
            // command-specific error codes (e.g. CS001 for the spec command).
            Ok(ArgValue::Enum(s.to_string()))
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
            },
        ]);

        let cmd = build_typed_clap_command("test", &spec);
        let arg_ids: Vec<_> = cmd.get_arguments().map(|a| a.get_id().as_str()).collect();
        assert!(arg_ids.contains(&"output"), "expected 'output' arg");
        assert!(arg_ids.contains(&"verbose"), "expected 'verbose' arg");
    }

    #[test]
    fn build_legacy_clap_command_has_trailing_vararg() {
        use crate::command::Command;
        use std::sync::Arc;

        let cmd = Command {
            id: "legacy",
            summary: "Legacy command",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
        };

        let clap_cmd = build_legacy_clap_command(&cmd);
        let arg_ids: Vec<_> = clap_cmd
            .get_arguments()
            .map(|a| a.get_id().as_str())
            .collect();

        #[cfg(not(feature = "strict-args"))]
        assert!(
            arg_ids.contains(&"trailing"),
            "expected 'trailing' var-arg when strict-args is disabled"
        );

        #[cfg(feature = "strict-args")]
        assert!(
            !arg_ids.contains(&"trailing"),
            "trailing var-arg should not be present when strict-args is enabled"
        );
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
