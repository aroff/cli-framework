//! Unit tests for HelpRenderer

use cli_framework::app::AppBuilder;
use cli_framework::app::AppContext;
use cli_framework::cli_output::HelpRenderer;
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::spec::command_tree::{CommandPath, CommandSpec, GroupMetadata};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_arc_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn AppContext,
            HashMap<String, cli_framework::spec::value::ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) }))
}

#[test]
fn t6_render_empty_registry_includes_options_block() {
    let registry = CommandRegistry::new();
    let output = HelpRenderer::new(None, &registry).render();

    assert!(!output.is_empty());
    assert!(output.contains("Options:\n"));
    assert!(output.contains("--help, -h"));
}

#[test]
fn t7_renders_sorted_categories_and_sorted_commands_within_group() {
    let mut registry = CommandRegistry::new();

    // Services group (intentionally registered out of order)
    registry.register(Command {
        id: Arc::from("stop"),
        spec: Arc::new(CommandSpec {
            summary: "Stop service",
            category: Some("Services"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: Arc::from("start"),
        spec: Arc::new(CommandSpec {
            summary: "Start service",
            category: Some("Services"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: Arc::from("restart"),
        spec: Arc::new(CommandSpec {
            summary: "Restart service",
            category: Some("Services"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_arc_execute(),
    });

    // Other groups
    registry.register(Command {
        id: Arc::from("metrics"),
        spec: Arc::new(CommandSpec {
            summary: "Show metrics",
            category: Some("Observability"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: Arc::from("whoami"),
        spec: Arc::new(CommandSpec {
            summary: "Print user",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_arc_execute(),
    });

    let output = HelpRenderer::new(None, &registry).render();

    let obs = output
        .find("Observability:")
        .expect("Observability heading");
    let svc = output.find("Services:").expect("Services heading");
    let other = output.find("Other:").expect("Other heading");
    assert!(obs < svc);
    assert!(svc < other);

    let restart = output.find("  restart").expect("restart line");
    let start = output.find("  start").expect("start line");
    let stop = output.find("  stop").expect("stop line");
    assert!(restart < start);
    assert!(start < stop);
}

#[test]
fn t8_renders_fixed_width_id_column_per_group() {
    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: Arc::from("logs"),
        spec: Arc::new(CommandSpec {
            summary: "Show logs",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: Arc::from("status"),
        spec: Arc::new(CommandSpec {
            summary: "Show status",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_arc_execute(),
    });

    let output = HelpRenderer::new(None, &registry).render();

    // col_width = max(len("logs"), len("status")) + 2 = 6 + 2 = 8
    // Ensure "status" is padded with 2 spaces before summary begins.
    assert!(output.contains("  status  Show status"));
}

#[test]
fn render_normalized_matches_render() {
    let registry = CommandRegistry::new();
    let renderer = HelpRenderer::new(None, &registry);
    assert_eq!(renderer.render(), renderer.render_normalized());
}

#[test]
fn categorized_groups_honor_explicit_section_and_entry_order_with_compact_rows() {
    let builder = AppBuilder::new().with_help_section_order(&[
        "Operations",
        "Skills and projects",
        "Operations",
    ]);
    assert_eq!(
        builder.command_registry().help_section_order(),
        &["Operations", "Skills and projects"]
    );
    let mut registry = builder.command_registry().clone();

    registry
        .register_group(
            &CommandPath::root_for("bundle"),
            GroupMetadata {
                summary: "Build and manage skill bundles",
                category: Some("Skills and projects"),
                help_order: Some(20),
                ..Default::default()
            },
        )
        .unwrap();
    registry
        .register_group(
            &CommandPath::root_for("skill"),
            GroupMetadata {
                summary: "Discover and manage skills",
                category: Some("Skills and projects"),
                help_order: Some(10),
                ..Default::default()
            },
        )
        .unwrap();
    registry
        .register_group(
            &CommandPath::root_for("server"),
            GroupMetadata {
                summary: "Run the HTTP API",
                category: Some("Operations"),
                help_order: Some(30),
                ..Default::default()
            },
        )
        .unwrap();
    registry.register(Command {
        id: Arc::from("doctor"),
        spec: Arc::new(CommandSpec {
            summary: "Diagnose setup",
            syntax: Some("doctor [--json]"),
            category: Some("Operations"),
            help_order: Some(10),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_arc_execute(),
    });

    let output = HelpRenderer::new(None, &registry).render();
    let operations = output.find("Operations:").unwrap();
    let skills = output.find("Skills and projects:").unwrap();
    let skill = output.find("  skill ").unwrap();
    let bundle = output.find("  bundle").unwrap();

    assert!(
        operations < skills,
        "explicit section order was ignored:\n{output}"
    );
    assert!(
        skill < bundle,
        "explicit entry order was ignored:\n{output}"
    );
    assert_eq!(
        output.matches("Usage:").count(),
        1,
        "root help must contain only its own usage line:\n{output}"
    );
    assert!(!output.contains("doctor [--json]"));
}

struct DummyCtx;
impl AppContext for DummyCtx {}

// AC-1: run_with_args(["app", "--help"]) with ≥2 distinct categories → HelpRenderer grouped output
#[cfg(feature = "testkit")]
#[tokio::test]
async fn t9_run_with_args_help_flag_routes_through_help_renderer_when_categories_present() {
    use cli_framework::testkit::CliTestHarness;

    let app = AppBuilder::new()
        .with_version("testapp", "1.0.0")
        .register_command(Command {
            id: Arc::from("deploy"),
            spec: Arc::new(CommandSpec {
                summary: "Deploy service",
                category: Some("Deployment"),
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .register_command(Command {
            id: Arc::from("rollback"),
            spec: Arc::new(CommandSpec {
                summary: "Rollback deployment",
                category: Some("Deployment"),
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .register_command(Command {
            id: Arc::from("logs"),
            spec: Arc::new(CommandSpec {
                summary: "Show logs",
                category: Some("Observability"),
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["testapp", "--help"]).await;

    assert!(
        out.stdout().contains("Deployment:"),
        "expected 'Deployment:' heading in output:\n{}",
        out.stdout()
    );
    assert!(
        out.stdout().contains("Observability:"),
        "expected 'Observability:' heading in output:\n{}",
        out.stdout()
    );
    let dep_pos = out.stdout().find("Deployment:").unwrap();
    let obs_pos = out.stdout().find("Observability:").unwrap();
    assert!(
        dep_pos < obs_pos,
        "Deployment heading should appear before Observability (alphabetical: D < O)"
    );
}

// AC-2: run_with_args(["app"]) (no subcommand) with categories → same grouped output
#[cfg(feature = "testkit")]
#[tokio::test]
async fn t10_run_with_args_no_subcommand_routes_through_help_renderer_when_categories_present() {
    use cli_framework::testkit::CliTestHarness;

    let app = AppBuilder::new()
        .with_version("testapp", "1.0.0")
        .register_command(Command {
            id: Arc::from("deploy"),
            spec: Arc::new(CommandSpec {
                summary: "Deploy service",
                category: Some("Deployment"),
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .register_command(Command {
            id: Arc::from("logs"),
            spec: Arc::new(CommandSpec {
                summary: "Show logs",
                category: Some("Observability"),
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["testapp"]).await;

    assert!(
        out.stdout().contains("Deployment:"),
        "expected 'Deployment:' heading in output:\n{}",
        out.stdout()
    );
    assert!(
        out.stdout().contains("Observability:"),
        "expected 'Observability:' heading in output:\n{}",
        out.stdout()
    );
}

#[cfg(feature = "testkit")]
#[tokio::test]
async fn categorized_root_group_enables_grouped_help_without_categorized_leaf_commands() {
    use cli_framework::testkit::CliTestHarness;

    let app = AppBuilder::new()
        .with_version("testapp", "1.0.0")
        .register_group(
            &CommandPath::root_for("skill"),
            GroupMetadata {
                summary: "Discover and manage skills",
                category: Some("Skills and projects"),
                ..Default::default()
            },
        )
        .unwrap()
        .register_command_at(
            &CommandPath::new(&["skill", "list"]).unwrap(),
            Command {
                id: Arc::from("list"),
                spec: Arc::new(CommandSpec {
                    summary: "List installed skills",
                    ..Default::default()
                }),
                validator: None,
                expose_mcp: false,
                expose_chat: true,
                meta: None,
                visibility: None,
                execute: noop_arc_execute(),
            },
        )
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["testapp", "--help"]).await;

    assert!(out.stdout().contains("Skills And Projects:"));
    assert!(out.stdout().contains("  skill  Discover and manage skills"));
    assert!(!out.stdout().contains("Usage: list"));
}

// AC-4: run_with_args(["app", "--help"]) with all category: None → clap flat output (no category headings)
#[cfg(feature = "testkit")]
#[tokio::test]
async fn t11_run_with_args_help_flag_uses_clap_when_no_categories() {
    use cli_framework::testkit::CliTestHarness;

    let app = AppBuilder::new()
        .with_version("testapp", "1.0.0")
        .register_command(Command {
            id: Arc::from("deploy"),
            spec: Arc::new(CommandSpec {
                summary: "Deploy service",
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .register_command(Command {
            id: Arc::from("logs"),
            spec: Arc::new(CommandSpec {
                summary: "Show logs",
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["testapp", "--help"]).await;

    // Clap flat output contains "Commands:" block, not grouped category headings
    assert!(
        out.stdout().contains("Commands:") || out.stdout().contains("Usage:"),
        "expected clap help output:\n{}",
        out.stdout()
    );
    assert!(
        !out.stdout().contains("Deployment:"),
        "should not contain category headings when no categories set:\n{}",
        out.stdout()
    );
}

// AC-5: render_help() must not contain the spurious version prefix line

#[test]
fn t12_render_help_does_not_contain_spurious_version_line() {
    let app = AppBuilder::new()
        .with_version("testapp", "1.0.0")
        .register_command(Command {
            id: Arc::from("deploy"),
            spec: Arc::new(CommandSpec {
                summary: "Deploy service",
                category: Some("Deployment"),
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let help = app.render_help();
    assert!(
        !help.contains("version - Print version information"),
        "render_help() must not contain spurious version prefix, got:\n{}",
        help
    );
}
