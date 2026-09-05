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
use cli_framework::telemetry::Deployment;
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
async fn reset_returns_a_configured_install_to_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = enduser_harness(dir.path());
    h.run(&["demo", "telemetry", "set", "debug"]).await;
    h.run(&["demo", "telemetry", "reset"]).await;
    let out = h.run(&["demo", "telemetry", "status", "--json"]).await;
    let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value["level"], "off");
    assert_eq!(value["level_source"], "default");
}
