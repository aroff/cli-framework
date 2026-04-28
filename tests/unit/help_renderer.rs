//! Unit tests for HelpRenderer

use cli_framework::app::AppContext;
use cli_framework::cli_output::HelpRenderer;
use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct TestContext;
impl AppContext for TestContext {}

fn noop_execute(
    _ctx: &mut dyn AppContext,
    _args: CommandArgs,
) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
    Box::pin(async move { Ok(()) })
}

fn noop_arc_execute() -> Arc<
    dyn Fn(
            &mut dyn AppContext,
            CommandArgs,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>
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
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: "start",
        summary: "Start service",
        syntax: None,
        category: Some("Services"),
        spec: None,
        validator: None,
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: "restart",
        summary: "Restart service",
        syntax: None,
        category: Some("Services"),
        spec: None,
        validator: None,
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
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: "whoami",
        summary: "Print user",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
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
        execute: noop_arc_execute(),
    });
    registry.register(Command {
        id: "status",
        summary: "Show status",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
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
