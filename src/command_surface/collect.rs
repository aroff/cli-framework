use crate::command::CommandRegistry;
use crate::command_surface::document::{
    CliSpecApp, CliSpecArg, CliSpecCommand, CliSpecDocument, CliSpecEnvVar, CliSpecExitCode,
};
use crate::command_surface::json_schema::build_input_schema;
use crate::spec::arg_spec::{ArgKind, ArgValueType, Cardinality};

/// Collect all commands from `registry` into a `CliSpecDocument`.
/// Commands with `CommandSpec.hidden == true` are excluded unless `include_hidden` is true.
pub fn collect(
    registry: &CommandRegistry,
    app_name: &str,
    app_version: &str,
    include_hidden: bool,
) -> CliSpecDocument {
    let mut commands: Vec<CliSpecCommand> = registry
        .all_tree_commands()
        .filter_map(|(path_str, cmd)| {
            if let Some(ref spec) = cmd.spec {
                if spec.hidden && !include_hidden {
                    return None;
                }
            }

            let hidden = cmd.spec.as_deref().map(|s| s.hidden).unwrap_or(false);
            let input_schema = build_input_schema(cmd.spec.as_deref());

            let (deprecated, aliases, args, examples, env_vars, exit_codes, notes) =
                if let Some(ref spec) = cmd.spec {
                    let args = spec
                        .args
                        .iter()
                        .map(|a| CliSpecArg {
                            name: a.name.to_string(),
                            kind: match a.kind {
                                ArgKind::Flag => "flag".to_string(),
                                ArgKind::Option => "option".to_string(),
                                ArgKind::Positional => "positional".to_string(),
                            },
                            short: a.short,
                            long: a.long.map(|s| s.to_string()),
                            value_type: match &a.value_type {
                                ArgValueType::Bool => "bool".to_string(),
                                ArgValueType::String => "string".to_string(),
                                ArgValueType::Int => "int".to_string(),
                                ArgValueType::Float => "float".to_string(),
                                ArgValueType::Enum(variants) => {
                                    format!("enum:{}", variants.join(","))
                                }
                            },
                            cardinality: match a.cardinality {
                                Cardinality::Required => "required".to_string(),
                                Cardinality::Optional => "optional".to_string(),
                                Cardinality::Repeated => "repeated".to_string(),
                            },
                            default: a.default.as_ref().map(|v| v.to_string()),
                            help: a.help.to_string(),
                        })
                        .collect();

                    let env_vars = spec
                        .env_vars
                        .iter()
                        .map(|e| CliSpecEnvVar {
                            name: e.name.to_string(),
                            description: e.description.to_string(),
                        })
                        .collect();

                    let exit_codes = spec
                        .exit_codes
                        .iter()
                        .map(|e| CliSpecExitCode {
                            code: e.code,
                            description: e.description.to_string(),
                        })
                        .collect();

                    (
                        spec.deprecated.map(|d| d.to_string()),
                        spec.aliases.iter().map(|a| a.to_string()).collect(),
                        args,
                        spec.examples.iter().map(|e| e.to_string()).collect(),
                        env_vars,
                        exit_codes,
                        spec.notes.map(|n| n.to_string()),
                    )
                } else {
                    (None, vec![], vec![], vec![], vec![], vec![], None)
                };

            Some(CliSpecCommand {
                path: path_str.to_string(),
                id: cmd.id.to_string(),
                summary: cmd.summary.to_string(),
                syntax: cmd.syntax.map(|s| s.to_string()),
                category: cmd.category.map(|c| c.to_string()),
                hidden,
                deprecated,
                aliases,
                args,
                input_schema,
                examples,
                env_vars,
                exit_codes,
                notes,
            })
        })
        .collect();

    commands.sort_by(|a, b| a.path.cmp(&b.path));

    CliSpecDocument {
        schema_version: "cli-framework.command-surface.v1",
        app: CliSpecApp {
            name: app_name.to_string(),
            version: app_version.to_string(),
        },
        commands,
    }
}
