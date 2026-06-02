//! Unit tests for HelpRenderer

use cli_framework::app::AppBuilder;
use cli_framework::app::AppContext;
use cli_framework::cli_output::HelpRenderer;
use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_arc_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn AppContext,
            CommandArgs,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx: &mut dyn AppContext, _args: CommandArgs| Box::pin(async move { Ok(()) }))
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
        id: "stop",
        summary: "Stop service",
        syntax: None,
        category: Some("Services"),
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: "start",
        summary: "Start service",
        syntax: None,
        category: Some("Services"),
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: "restart",
        summary: "Restart service",
        syntax: None,
        category: Some("Services"),
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: noop_arc_execute(),
    });

    // Other groups
    registry.register(Command {
        id: "metrics",
        summary: "Show metrics",
        syntax: None,
        category: Some("Observability"),
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: "whoami",
        summary: "Print user",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
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
        id: "logs",
        summary: "Show logs",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: "status",
        summary: "Show status",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
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

struct DummyCtx;
impl AppContext for DummyCtx {}

// AC-1: run_with_args(["app", "--help"]) with ≥2 distinct categories → HelpRenderer grouped output
#[cfg(all(feature = "testkit", not(feature = "strict-types")))]
#[tokio::test]
async fn t9_run_with_args_help_flag_routes_through_help_renderer_when_categories_present() {
    use cli_framework::testkit::CliTestHarness;

    let app = AppBuilder::new()
        .with_version("testapp", "1.0.0")
        .register_command(Command {
            id: "deploy",
            summary: "Deploy service",
            syntax: None,
            category: Some("Deployment"),
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .register_command(Command {
            id: "rollback",
            summary: "Rollback deployment",
            syntax: None,
            category: Some("Deployment"),
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .register_command(Command {
            id: "logs",
            summary: "Show logs",
            syntax: None,
            category: Some("Observability"),
            spec: None,
            validator: None,
            expose_mcp: false,
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
        obs_pos < dep_pos,
        "Observability heading should appear before Deployment (alphabetical)"
    );
}

// AC-2: run_with_args(["app"]) (no subcommand) with categories → same grouped output
#[cfg(all(feature = "testkit", not(feature = "strict-types")))]
#[tokio::test]
async fn t10_run_with_args_no_subcommand_routes_through_help_renderer_when_categories_present() {
    use cli_framework::testkit::CliTestHarness;

    let app = AppBuilder::new()
        .with_version("testapp", "1.0.0")
        .register_command(Command {
            id: "deploy",
            summary: "Deploy service",
            syntax: None,
            category: Some("Deployment"),
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .register_command(Command {
            id: "logs",
            summary: "Show logs",
            syntax: None,
            category: Some("Observability"),
            spec: None,
            validator: None,
            expose_mcp: false,
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

// AC-4: run_with_args(["app", "--help"]) with all category: None → clap flat output (no category headings)
#[cfg(all(feature = "testkit", not(feature = "strict-types")))]
#[tokio::test]
async fn t11_run_with_args_help_flag_uses_clap_when_no_categories() {
    use cli_framework::testkit::CliTestHarness;

    let app = AppBuilder::new()
        .with_version("testapp", "1.0.0")
        .register_command(Command {
            id: "deploy",
            summary: "Deploy service",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: noop_arc_execute(),
        })
        .unwrap()
        .register_command(Command {
            id: "logs",
            summary: "Show logs",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
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
#[cfg(not(feature = "strict-types"))]
#[test]
fn t12_render_help_does_not_contain_spurious_version_line() {
    let app = AppBuilder::new()
        .with_version("testapp", "1.0.0")
        .register_command(Command {
            id: "deploy",
            summary: "Deploy service",
            syntax: None,
            category: Some("Deployment"),
            spec: None,
            validator: None,
            expose_mcp: false,
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
