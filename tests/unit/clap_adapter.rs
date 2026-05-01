use cli_framework::app::AppMeta;
use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use cli_framework::parser::ParseOutcome;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct TestContext;
impl cli_framework::app::AppContext for TestContext {}

fn noop_execute() -> Arc<
    dyn Fn(
            &mut dyn cli_framework::app::AppContext,
            CommandArgs,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

fn make_registry_with(commands: Vec<Command>) -> CommandRegistry {
    let mut reg = CommandRegistry::new();
    for cmd in commands {
        reg.register(cmd);
    }
    reg
}

#[test]
fn build_clap_root_subcommand_count_matches_registry() {
    let registry = make_registry_with(vec![
        Command {
            id: "hello",
            summary: "Say hello",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            execute: noop_execute(),
        },
        Command {
            id: "goodbye",
            summary: "Say goodbye",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            execute: noop_execute(),
        },
    ]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let matches = root
        .clone()
        .try_get_matches_from(["testapp", "--help"])
        .unwrap_err();
    let help_output = matches.to_string();
    assert!(help_output.contains("hello"));
    assert!(help_output.contains("goodbye"));
}

#[test]
fn build_clap_root_version_propagation() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "myapp",
        "2.0.0",
    );

    let err = root
        .clone()
        .try_get_matches_from(["myapp", "--version"])
        .unwrap_err();
    let output = err.to_string();
    assert!(output.contains("myapp"));
    assert!(output.contains("2.0.0"));
}

#[test]
fn build_clap_root_override_usage_from_meta() {
    let meta = AppMeta {
        name: "myapp",
        version: "1.0.0",
        description: "A test app",
        usage: Some("myapp custom-usage <cmd>"),
    };

    let root = cli_framework::app::clap_adapter::build_clap_root(
        Some(&meta),
        &CommandRegistry::new(),
        "myapp",
        "1.0.0",
    );

    let err = root
        .clone()
        .try_get_matches_from(["myapp", "--help"])
        .unwrap_err();
    let help_output = err.to_string();
    assert!(help_output.contains("myapp custom-usage <cmd>"));
}

#[test]
fn build_clap_root_description_from_meta() {
    let meta = AppMeta {
        name: "myapp",
        version: "1.0.0",
        description: "An amazing application",
        usage: None,
    };

    let root = cli_framework::app::clap_adapter::build_clap_root(
        Some(&meta),
        &CommandRegistry::new(),
        "myapp",
        "1.0.0",
    );

    let err = root
        .clone()
        .try_get_matches_from(["myapp", "--help"])
        .unwrap_err();
    let help_output = err.to_string();
    assert!(help_output.contains("An amazing application"));
}

#[test]
fn build_clap_root_adds_version_subcommand_when_not_registered() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "testapp",
        "0.1.0",
    );

    let matches = root
        .clone()
        .try_get_matches_from(["testapp", "version"])
        .unwrap();
    let (name, _) = matches.subcommand().unwrap();
    assert_eq!(name, "version");
}

#[test]
fn build_clap_root_no_version_subcommand_when_user_registers_version() {
    let registry = make_registry_with(vec![Command {
        id: "version",
        summary: "Custom version",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let matches = root
        .clone()
        .try_get_matches_from(["testapp", "version"])
        .unwrap();
    let (name, sub_matches) = matches.subcommand().unwrap();
    assert_eq!(name, "version");
    // strict-args disables trailing vararg, so the "trailing" id is not registered
    #[cfg(not(feature = "strict-args"))]
    {
        let trailing: Vec<String> = sub_matches
            .get_many::<String>("trailing")
            .map(|v| v.cloned().collect())
            .unwrap_or_default();
        assert!(trailing.is_empty());
    }
    #[cfg(feature = "strict-args")]
    let _ = sub_matches;
}

#[test]
#[cfg(not(feature = "strict-args"))]
fn parse_with_clap_key_value() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name".to_string(),
            "Alice".to_string(),
        ],
    );

    match outcome {
        ParseOutcome::Parsed {
            command_path, args, ..
        } => {
            assert_eq!(command_path.leaf().unwrap(), "hello");
            assert_eq!(args.named.get("name").unwrap(), "Alice");
            assert!(args.positional.is_empty());
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]
#[cfg(not(feature = "strict-args"))]
fn parse_with_clap_key_equals_value() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name=Alice".to_string(),
        ],
    );

    match outcome {
        ParseOutcome::Parsed {
            command_path, args, ..
        } => {
            assert_eq!(command_path.leaf().unwrap(), "hello");
            assert_eq!(args.named.get("name").unwrap(), "Alice");
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]
#[cfg(not(feature = "strict-args"))]
fn parse_with_clap_positional_after_double_dash() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--".to_string(),
            "positional".to_string(),
        ],
    );

    match outcome {
        ParseOutcome::Parsed {
            command_path, args, ..
        } => {
            assert_eq!(command_path.leaf().unwrap(), "hello");
            assert!(args.positional.contains(&"positional".to_string()));
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]
fn parse_with_clap_help_returns_help_shown() {
    let registry = CommandRegistry::new();
    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "--help".to_string()],
    );

    assert!(
        matches!(outcome, ParseOutcome::HelpShown(_)),
        "expected HelpShown, got {:?}",
        outcome
    );
}

#[test]
fn parse_with_clap_version_flag_returns_version_shown() {
    let registry = CommandRegistry::new();
    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "--version".to_string()],
    );

    assert!(
        matches!(outcome, ParseOutcome::VersionShown(_)),
        "expected VersionShown, got {:?}",
        outcome
    );
}

#[test]
fn parse_with_clap_unknown_command_returns_parse_error() {
    let registry = CommandRegistry::new();
    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "nonexistent".to_string()],
    );

    assert!(
        matches!(outcome, ParseOutcome::ParseError(_)),
        "expected ParseError, got {:?}",
        outcome
    );
}

#[test]
fn parse_with_clap_version_subcommand_returns_parsed() {
    let registry = CommandRegistry::new();
    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "version".to_string()],
    );

    match outcome {
        ParseOutcome::Parsed { command_path, .. } => {
            assert_eq!(command_path.leaf().unwrap(), "version");
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]
#[cfg(not(feature = "strict-args"))]
fn parse_with_clap_mixed_positional_and_named() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "file.txt".to_string(),
            "--name".to_string(),
            "Alice".to_string(),
        ],
    );

    match outcome {
        ParseOutcome::Parsed {
            command_path, args, ..
        } => {
            assert_eq!(command_path.leaf().unwrap(), "hello");
            assert_eq!(args.positional, vec!["file.txt"]);
            assert_eq!(args.named.get("name").unwrap(), "Alice");
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

// DD#8: bare --flag without a value must NOT insert "true" into named args.
#[test]
#[cfg(not(feature = "strict-args"))]
fn parse_with_clap_bare_flag_not_inserted_as_true() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--verbose".to_string(),
        ],
    );

    match outcome {
        ParseOutcome::Parsed {
            command_path, args, ..
        } => {
            assert_eq!(command_path.leaf().unwrap(), "hello");
            assert!(
                args.named.get("verbose").is_none(),
                "bare --flag must NOT appear in named (DD#8)"
            );
            assert!(
                !args.positional.contains(&"--verbose".to_string()),
                "bare --flag must NOT appear in positional"
            );
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

// Verify that --key value and --key=value produce identical CommandArgs (AC-G1.2).
#[test]
#[cfg(not(feature = "strict-args"))]
fn parse_with_clap_key_value_and_equals_value_parity() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome_space = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name".to_string(),
            "Alice".to_string(),
        ],
    );

    let outcome_eq = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name=Alice".to_string(),
        ],
    );

    match (outcome_space, outcome_eq) {
        (
            ParseOutcome::Parsed {
                command_path: p1,
                args: a1,
                ..
            },
            ParseOutcome::Parsed {
                command_path: p2,
                args: a2,
                ..
            },
        ) => {
            assert_eq!(p1.leaf().unwrap(), p2.leaf().unwrap());
            assert_eq!(a1.named, a2.named);
            assert_eq!(a1.positional, a2.positional);
        }
        other => panic!("expected both Parsed, got {:?}", other),
    }
}
