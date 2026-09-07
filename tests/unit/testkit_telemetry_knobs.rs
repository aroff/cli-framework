// tests/unit/testkit_telemetry_knobs.rs
//! Testkit knobs for the spec 025 first-run notice (Task 29).
//!
//! The tty knob is what makes the notice testable. Without it a notice test
//! passes or fails depending on whether the developer ran `cargo test` in a
//! terminal and CI ran it in a pipe, which is the definition of a flaky
//! test. So the harness declares the answer instead of asking the process,
//! and declares it *false* by default: a test that only passes in a terminal
//! is worse than no test.
//!
//! There is no `testkit::Harness` and no `with_isolated_config_dir()` — the
//! real harness is [`cli_framework::testkit::CliTestHarness<C>`], which wraps
//! an already-built [`App`]. Store isolation therefore happens *before*
//! build, via [`AppBuilder::with_telemetry_config_dir`], exactly as
//! `tests/integration/telemetry_cli.rs` established in PR6. Nothing here
//! touches `XDG_CONFIG_HOME` or any other process-global state, so these
//! tests need no serialization against each other.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::telemetry::Deployment;
use cli_framework::testkit::CliTestHarness;
use std::path::Path;

struct TestCtx;
impl AppContext for TestCtx {}

/// The first line of notice template 1. Asserting on the real sentence
/// rather than the bare word "telemetry" keeps the test honest: `--help`
/// output on an app with a `telemetry` command group contains that word
/// anyway, so a substring test for it would pass with the notice deleted.
const NOTICE_LINE: &str = "usage statistics are off";

/// An end-user app whose telemetry settings file lives under `dir`.
fn harness(dir: &Path) -> CliTestHarness<TestCtx> {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::EndUser { privacy_url: None })
        .with_telemetry_config_dir(dir)
        .build(TestCtx)
        .unwrap();
    CliTestHarness::new(app)
}

#[tokio::test]
async fn declaring_stderr_interactive_makes_the_notice_appear() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path()).with_interactive_stderr(true);
    let out = h.run(&["demo", "--help"]).await;
    assert!(
        out.stderr.contains(NOTICE_LINE),
        "a harness that declares stderr interactive must see the first-run \
         notice; got stderr: {:?}",
        out.stderr
    );
}

#[tokio::test]
async fn the_default_is_not_interactive_so_a_test_never_depends_on_how_it_was_launched() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());
    let out = h.run(&["demo", "--help"]).await;
    assert!(
        !out.stderr.contains(NOTICE_LINE),
        "a notice test that passes in a terminal and fails in CI is the \
         definition of a flaky test: {:?}",
        out.stderr
    );
}

#[tokio::test]
async fn each_harness_gets_its_own_config_directory() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let mut a = harness(dir_a.path());
    let mut b = harness(dir_b.path());

    a.run(&["demo", "telemetry", "set", "debug"]).await;

    let out = b.run(&["demo", "telemetry", "status", "--json"]).await;
    let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(
        value["level"], "off",
        "harnesses must not see each other's state: {}",
        out.stdout
    );
}

#[tokio::test]
async fn the_isolated_directory_is_not_the_developers_real_config_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());
    h.run(&["demo", "telemetry", "set", "usage"]).await;

    // The settings file has to land under the isolated directory, because
    // the alternative is that it landed under the person's real one. A test
    // that mutates someone's actual telemetry consent is a bug that ships,
    // and it ships silently: the suite stays green either way.
    let settled = dir.path().join("demo").join("telemetry.json");
    assert!(
        settled.is_file(),
        "`with_telemetry_config_dir` was ignored: nothing at {settled:?}"
    );
    let body = std::fs::read_to_string(&settled).unwrap();
    assert!(
        body.contains("usage"),
        "the isolated file exists but is not the one the command wrote: {body}"
    );

    let real = dirs::config_dir().unwrap_or_default();
    assert!(
        real.as_os_str().is_empty() || !dir.path().starts_with(&real),
        "the isolated directory {:?} is inside the real configuration \
         directory {real:?}",
        dir.path()
    );
}

#[tokio::test]
async fn the_notice_appears_once_across_two_runs_of_the_same_harness() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path()).with_interactive_stderr(true);

    let first = h.run(&["demo", "--help"]).await;
    let second = h.run(&["demo", "--help"]).await;

    assert!(
        first.stderr.contains(NOTICE_LINE),
        "first run: {:?}",
        first.stderr
    );
    assert!(
        !second.stderr.contains(NOTICE_LINE),
        "the notice records what it announced, so the second run is quiet: {:?}",
        second.stderr
    );
}
