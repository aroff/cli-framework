// tests/integration/telemetry_cli.rs
//! End-to-end behaviour of the `telemetry` command group (Task 23), driven
//! through a real [`App`] and [`CliTestHarness`] rather than the pure
//! functions `tests/unit/telemetry_commands.rs` exercises directly.
//!
//! There is no `testkit::Harness` and no `with_isolated_config_dir()` — the
//! real harness is [`cli_framework::testkit::CliTestHarness<C>`], which wraps
//! an already-built [`App`]. Store isolation therefore has to happen
//! *before* build, via [`AppBuilder::with_telemetry_config_dir`], which
//! points the telemetry settings file at a per-test [`tempfile::TempDir`]
//! instead of the platform configuration directory. This never touches
//! `XDG_CONFIG_HOME` or any other process-global environment state, so tests
//! in this file need no serialization against each other.
//!
//! Every test name and assertion below is unchanged from the plan; only the
//! harness construction differs.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::telemetry::{Deployment, TelemetryStore};
use cli_framework::testkit::CliTestHarness;
use std::path::Path;

struct TestCtx;
impl AppContext for TestCtx {}

/// Build an app with `deployment`, its telemetry settings file isolated
/// under `dir`. `app_name` is fixed at `"demo"` throughout this file: it is
/// both the name `AppBuilder::build` passes to `register_telemetry_commands`
/// and the subdirectory the settings file lives under (`<dir>/demo/...`), so
/// every test opens the same store it isolated.
fn harness(deployment: Deployment, dir: &Path) -> CliTestHarness<TestCtx> {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(deployment)
        .with_telemetry_config_dir(dir)
        .build(TestCtx)
        .unwrap();
    CliTestHarness::new(app)
}

fn enduser_harness(dir: &Path) -> CliTestHarness<TestCtx> {
    harness(Deployment::EndUser { privacy_url: None }, dir)
}

#[tokio::test]
async fn a_fresh_install_reports_telemetry_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    let out = h.run(&["demo", "telemetry", "status"]).await;
    assert!(out.stdout.contains("off"), "{}", out.stdout);
    assert_eq!(out.exit_code, 0);
}

#[tokio::test]
async fn a_person_can_turn_telemetry_on_and_see_it_stick_across_processes() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    h.run(&["demo", "telemetry", "set", "usage"]).await;
    let out = h.run(&["demo", "telemetry", "status", "--json"]).await;
    let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value["level"], "usage");
    assert_eq!(value["level_source"], "config_file");
}

#[tokio::test]
async fn the_json_output_is_the_only_thing_on_stdout_so_it_can_be_piped() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    let out = h.run(&["demo", "telemetry", "status", "--json"]).await;
    serde_json::from_str::<serde_json::Value>(&out.stdout)
        .expect("stdout is a single JSON document with no banner or warning mixed in");
}

#[tokio::test]
async fn telemetry_info_lists_the_whole_catalog_with_telemetry_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    let out = h.run(&["demo", "telemetry", "info"]).await;
    for probe in ["cli.command", "cli.panic", "http.client", "mcp.session"] {
        assert!(
            out.stdout.contains(probe),
            "{probe} missing from info output"
        );
    }
    assert_eq!(
        out.exit_code, 0,
        "info answers 'what could this ever send' and must work when off"
    );
}

#[tokio::test]
async fn telemetry_info_as_json_lists_the_catalog_with_the_documented_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    let out = h.run(&["demo", "telemetry", "info", "--json"]).await;
    let value: serde_json::Value = serde_json::from_str(&out.stdout)
        .expect("info --json prints a single JSON document, same contract as status --json");
    let catalog = value.as_array().expect("the catalog is a JSON array");
    assert!(!catalog.is_empty(), "an empty catalog cannot be right");
    let command = catalog
        .iter()
        .find(|p| p["id"] == "cli.command")
        .expect("cli.command missing from the JSON catalog");
    assert_eq!(command["min_level"], "usage");
    assert!(command["summary"].is_string());
    assert!(command["sends"].is_string());
    // Spec 025 line 492 documents six fields, not four: `enabled` and
    // `effective now` must be present alongside id/min_level/summary/sends,
    // computed the same way `telemetry status`'s catalog is.
    assert_eq!(
        command["enabled"], true,
        "no probe has been disabled in this fixture"
    );
    assert_eq!(
        command["effective"], false,
        "this is a fresh install: telemetry defaults to off, and cli.command \
         needs usage, so it is enabled but not currently effective"
    );
    assert_eq!(out.exit_code, 0);
}

#[tokio::test]
async fn a_service_deployment_has_no_telemetry_command_group_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(Deployment::Service, dir.path());
    let out = h.run(&["demo", "telemetry", "status"]).await;
    assert_ne!(out.exit_code, 0);
    // The plan predicted a top-level "unrecognized subcommand" (E001) message
    // here; the framework actually reports "nested command path '...' not
    // found" (E012, `E_NESTED_COMMAND_NOT_FOUND`) for a multi-segment path
    // whose parent segment was never registered — confirmed empirically by
    // running this test against the real dispatcher. Both codes mean the
    // same thing from a caller's point of view ("this command does not
    // exist"), so the disjunction is widened rather than narrowed: a real
    // regression that made the group dispatch succeed, or changed the
    // wording of both known error paths, would still fail this assertion.
    assert!(
        out.stderr.contains("unrecognized")
            || out.stderr.contains("unknown")
            || out.stderr.contains("not found"),
        "on a server the level is a config decision; a runtime toggle would be \
         a second source of truth: {}",
        out.stderr
    );
}

#[tokio::test]
async fn an_invalid_level_names_the_valid_ones() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    let out = h.run(&["demo", "telemetry", "set", "verbose"]).await;
    assert_ne!(out.exit_code, 0);
    for level in ["off", "usage", "diagnostic", "debug"] {
        assert!(
            out.stderr.contains(level),
            "{level} missing from the error: {}",
            out.stderr
        );
    }
}

#[tokio::test]
async fn disabling_a_probe_shows_it_disabled_in_status() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    h.run(&["demo", "telemetry", "set", "diagnostic"]).await;
    h.run(&["demo", "telemetry", "disable", "cli.command.args"])
        .await;
    let out = h.run(&["demo", "telemetry", "status", "--json"]).await;
    let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let probe = value["probes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "cli.command.args")
        .unwrap();
    assert_eq!(probe["enabled"], false);
    assert_eq!(probe["effective"], false);
}

#[tokio::test]
async fn enabling_a_previously_disabled_probe_shows_it_effective_again_in_status() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    h.run(&["demo", "telemetry", "set", "diagnostic"]).await;
    h.run(&["demo", "telemetry", "disable", "cli.command.args"])
        .await;
    let out = h
        .run(&["demo", "telemetry", "enable", "cli.command.args"])
        .await;
    assert!(out.stdout.contains("cli.command.args"), "{}", out.stdout);
    assert!(out.stdout.contains("enabled"), "{}", out.stdout);
    assert_eq!(out.exit_code, 0);
    let status = h.run(&["demo", "telemetry", "status", "--json"]).await;
    let value: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
    let probe = value["probes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "cli.command.args")
        .unwrap();
    assert_eq!(probe["enabled"], true);
    assert_eq!(probe["effective"], true);
}

#[tokio::test]
async fn enabling_or_disabling_an_unknown_probe_id_is_a_reported_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    for verb in ["disable", "enable"] {
        let out = h.run(&["demo", "telemetry", verb, "cli.nonexistent"]).await;
        assert_ne!(
            out.exit_code, 0,
            "{verb} on an unknown probe must fail, not silently no-op: {}",
            out.stdout
        );
        assert!(
            out.stderr.contains("cli.nonexistent"),
            "{verb}: {}",
            out.stderr
        );
    }
}

#[tokio::test]
async fn reset_returns_a_configured_install_to_a_brand_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    h.run(&["demo", "telemetry", "set", "debug"]).await;
    // Startup mints the id (PR7 wires that); nothing inside the `telemetry`
    // group does, so this test stands in for the run that would have minted
    // one. Without it there is no id for `reset` to have to forget, and the
    // assertion below would pass against a `reset` that kept it.
    let before = TelemetryStore::open_at(dir.path(), "demo")
        .ensure_install_id()
        .unwrap();

    let reset_out = h.run(&["demo", "telemetry", "reset"]).await;
    assert_eq!(reset_out.exit_code, 0, "{}", reset_out.stderr);

    let out = h.run(&["demo", "telemetry", "status", "--json"]).await;
    let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value["level"], "off");
    assert_eq!(value["level_source"], "default");
    assert!(
        value["install_id"].is_null(),
        "status must not still report the id reset deleted: {}",
        out.stdout
    );
    assert!(
        !dir.path().join("demo").join("telemetry.json").exists(),
        "ADR 0077: reset deletes the framework-owned telemetry file"
    );
    assert_ne!(
        TelemetryStore::open_at(dir.path(), "demo")
            .ensure_install_id()
            .unwrap(),
        before,
        "the next run is a new Install with a new id, not the old one restored"
    );
}

// ── appended to tests/integration/telemetry_cli.rs ────────────────────────────
// Second pass (PR6-5 / PR6-6). Everything above drives the `telemetry` command
// group; nothing above ever touches the builder surface that *creates* it. The
// four tests below close that gap:
//
//   * `App::deployment()` and `AppBuilder::deployment()` — both added by this
//     PR with zero call sites anywhere in `src/`, `tests/` or `examples/`, so
//     neither `clippy -D warnings` (they are `pub`, so no dead-code lint) nor
//     line coverage (nothing calls them) could report them.
//   * the PRD's default deployment, asserted nowhere today.
//   * the "app already owns `telemetry`" guard, whose `else` arm had no test —
//     and which, as written, only catches one of the two shapes an app can own
//     that name in.

use cli_framework::command::Command;
use cli_framework::spec::command_tree::{CommandPath, CommandSpec, GroupMetadata};
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

type Execute = Arc<
    dyn for<'a> Fn(
            &'a mut dyn AppContext,
            HashMap<String, ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
>;

fn noop_execute() -> Execute {
    Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) }))
}

/// A command an app author registered themselves, under a name the framework
/// also wants.
fn app_command(id: &str, summary: &'static str) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary,
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: noop_execute(),
    }
}

#[test]
fn the_default_deployment_is_end_user_so_a_derived_app_gets_the_surface_for_free() {
    let dir = tempfile::tempdir().unwrap();
    // Deliberately no `.with_deployment(..)` — this asserts the default.
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_config_dir(dir.path())
        .build(TestCtx)
        .unwrap();
    assert_eq!(
        app.deployment(),
        &Deployment::EndUser { privacy_url: None },
        "spec 025: EndUser is the default. An author opts *into* Service; a CLI \
         that forgets to call with_deployment must still get the consent surface"
    );
    assert!(
        app.command_registry()
            .resolve(&CommandPath::new(&["telemetry", "status"]).unwrap())
            .is_some(),
        "and the default must actually produce the end-user command group"
    );
}

#[test]
fn the_builder_reads_back_the_deployment_it_was_given() {
    let builder = AppBuilder::new().with_version("demo", "0.0.0");
    assert_eq!(
        builder.deployment(),
        &Deployment::EndUser { privacy_url: None },
        "the builder's default must match the App's"
    );

    let builder = builder.with_deployment(Deployment::EndUser {
        privacy_url: Some("https://example.invalid/privacy".to_string()),
    });
    assert_eq!(
        builder.deployment().privacy_url(),
        Some("https://example.invalid/privacy"),
        "the first-run notice renders `Details: <url>` from exactly this value"
    );

    let builder = builder.with_deployment(Deployment::Service);
    assert_eq!(
        builder.deployment(),
        &Deployment::Service,
        "with_deployment replaces rather than merges"
    );
}

#[test]
fn an_app_that_already_owns_a_telemetry_command_keeps_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_config_dir(dir.path())
        .register_command(app_command("telemetry", "the app's own telemetry surface"))
        .unwrap()
        .build(TestCtx)
        .expect("an app that already owns `telemetry` must still build");

    assert_eq!(
        app.command_registry()
            .get("telemetry")
            .map(|c| c.spec.summary),
        Some("the app's own telemetry surface"),
        "the framework must not overwrite a command the author registered"
    );
    assert!(
        app.command_registry()
            .resolve(&CommandPath::new(&["telemetry", "status"]).unwrap())
            .is_none(),
        "nor graft its six subcommands underneath the author's command"
    );
}

#[test]
fn an_app_that_already_owns_a_telemetry_group_keeps_its_own() {
    // The shape an author is *most likely* to have: not a root command called
    // `telemetry`, but a group, because that is what `myapp telemetry export`
    // is. `CommandRegistry::get` reads only `tree_commands`, so the guard in
    // `AppBuilder::build` does not see a group and the framework tries to
    // register its own group on top — `register_group` collides on either map
    // and `build()` fails. Both shapes must reach the same warn-and-skip path.
    let dir = tempfile::tempdir().unwrap();
    let group = CommandPath::root_for("telemetry");
    let export = CommandPath::new(&["telemetry", "export"]).unwrap();

    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_config_dir(dir.path())
        .register_group(
            &group,
            GroupMetadata {
                summary: "the app's own telemetry surface",
                hidden: false,
            },
        )
        .unwrap()
        .register_command_at(&export, app_command("export", "export collected telemetry"))
        .unwrap()
        .build(TestCtx)
        .expect(
            "an app whose own `telemetry` surface is a group must still build: \
             the framework stands down, it does not collide",
        );

    assert!(
        app.command_registry().resolve(&export).is_some(),
        "the author's own subcommand must survive"
    );
    assert!(
        app.command_registry()
            .resolve(&CommandPath::new(&["telemetry", "status"]).unwrap())
            .is_none(),
        "and the framework's must not appear beside it"
    );
}
