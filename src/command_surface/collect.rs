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
            let spec = &*cmd.spec;

            if spec.hidden && !include_hidden {
                return None;
            }

            let input_schema = build_input_schema(Some(spec));

            let args: Vec<CliSpecArg> = spec
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
                    long: a.long.map(|s: &str| s.to_string()),
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

            let env_vars: Vec<CliSpecEnvVar> = spec
                .env_vars
                .iter()
                .map(|e| CliSpecEnvVar {
                    name: e.name.to_string(),
                    description: e.description.to_string(),
                })
                .collect();

            let exit_codes: Vec<CliSpecExitCode> = spec
                .exit_codes
                .iter()
                .map(|e| CliSpecExitCode {
                    code: e.code,
                    description: e.description.to_string(),
                })
                .collect();

            Some(CliSpecCommand {
                path: path_str.to_string(),
                id: cmd.id.to_string(),
                summary: cmd.summary().to_string(),
                syntax: cmd.syntax().map(|s: &str| s.to_string()),
                category: cmd.category().map(|c: &str| c.to_string()),
                hidden: spec.hidden,
                deprecated: spec.deprecated.map(|d: &str| d.to_string()),
                aliases: spec.aliases.iter().map(|a: &&str| a.to_string()).collect(),
                args,
                input_schema,
                examples: spec.examples.iter().map(|e: &&str| e.to_string()).collect(),
                env_vars,
                exit_codes,
                notes: spec.notes.map(|n: &str| n.to_string()),
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
