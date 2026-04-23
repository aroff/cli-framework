use cli_framework::app::AppMeta;
use cli_framework::command::{Command, CommandArgs, CommandRegistry};
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
            execute: noop_execute(),
        },
        Command {
            id: "goodbye",
            summary: "Say goodbye",
            syntax: None,
            category: None,
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
    let trailing: Vec<String> = sub_matches
        .get_many::<String>("trailing")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    assert!(trailing.is_empty());
}

#[test]
fn parse_with_clap_key_value() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let result = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name".to_string(),
            "Alice".to_string(),
        ],
    )
    .unwrap()
    .unwrap();

    assert_eq!(result.command_id, "hello");
    assert_eq!(result.args.named.get("name").unwrap(), "Alice");
    assert!(result.args.positional.is_empty());
}

#[test]
fn parse_with_clap_key_equals_value() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let result = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name=Alice".to_string(),
        ],
    )
    .unwrap()
    .unwrap();

    assert_eq!(result.command_id, "hello");
    assert_eq!(result.args.named.get("name").unwrap(), "Alice");
}

#[test]
fn parse_with_clap_positional_after_double_dash() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let result = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--".to_string(),
            "positional".to_string(),
        ],
    )
    .unwrap()
    .unwrap();

    assert_eq!(result.command_id, "hello");
    assert!(result.args.positional.contains(&"positional".to_string()));
}

#[test]
fn parse_with_clap_help_returns_none() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "testapp",
        "0.1.0",
    );

    let result = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        vec!["testapp".to_string(), "--help".to_string()],
    )
    .unwrap();

    assert!(result.is_none());
}

#[test]
fn parse_with_clap_version_flag_returns_none() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "testapp",
        "0.1.0",
    );

    let result = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        vec!["testapp".to_string(), "--version".to_string()],
    )
    .unwrap();

    assert!(result.is_none());
}

#[test]
fn parse_with_clap_unknown_command_returns_none() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "testapp",
        "0.1.0",
    );

    let result = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        vec!["testapp".to_string(), "nonexistent".to_string()],
    )
    .unwrap();

    assert!(result.is_none());
}

#[test]
fn parse_with_clap_version_subcommand_returns_some() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "testapp",
        "0.1.0",
    );

    let result = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        vec!["testapp".to_string(), "version".to_string()],
    )
    .unwrap()
    .unwrap();

    assert_eq!(result.command_id, "version");
}

#[test]
fn parse_with_clap_mixed_positional_and_named() {
    let registry = make_registry_with(vec![Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        execute: noop_execute(),
    }]);

    let root =
        cli_framework::app::clap_adapter::build_clap_root(None, &registry, "testapp", "0.1.0");

    let result = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "file.txt".to_string(),
            "--name".to_string(),
            "Alice".to_string(),
        ],
    )
    .unwrap()
    .unwrap();

    assert_eq!(result.command_id, "hello");
    assert_eq!(result.args.positional, vec!["file.txt"]);
    assert_eq!(result.args.named.get("name").unwrap(), "Alice");
}
