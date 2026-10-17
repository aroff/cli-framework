// tests/unit/end_user_telemetry_opt_out.rs
//! `AppBuilder::without_end_user_telemetry`: an end-user install the author
//! opted out of telemetry for gets no `telemetry` command group, no telemetry
//! doctor checks, no first-run notice and no export, while a `Service`
//! process of the same app keeps its telemetry.
//!
//! The use is a binary that is both a service and a program a person runs:
//! it picks its `Deployment` per process and wants telemetry only from the
//! service. Every test here has a control -- the same app without the opt-out
//! -- so a test cannot pass because the thing it looks for was never there.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::doctor::builtin::TmpdirWritableCheck;
use cli_framework::telemetry::{Deployment, TelemetryLevel};
use cli_framework::testkit::CliTestHarness;
use std::path::Path;
use std::sync::Arc;

struct TestCtx;
impl AppContext for TestCtx {}

/// The first line of the first-run notice; see `testkit_telemetry_knobs.rs`
/// for why the sentence and not the word "telemetry".
const NOTICE_LINE: &str = "usage statistics are off";

fn builder(dir: &Path, deployment: Deployment, opt_out: bool) -> AppBuilder {
    // One app check, so `doctor` exists whatever happens to the six
    // telemetry checks: an app with none at all gets no `doctor` command.
    let b = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(deployment)
        .with_telemetry_config_dir(dir)
        .register_doctor_checks(vec![Arc::new(TmpdirWritableCheck)]);
    if opt_out {
        b.without_end_user_telemetry()
    } else {
        b
    }
}

fn end_user() -> Deployment {
    Deployment::EndUser { privacy_url: None }
}

fn harness(dir: &Path, deployment: Deployment, opt_out: bool) -> CliTestHarness<TestCtx> {
    CliTestHarness::new(builder(dir, deployment, opt_out).build(TestCtx).unwrap())
}

fn has_telemetry_group(dir: &Path, deployment: Deployment, opt_out: bool) -> bool {
    let app = builder(dir, deployment, opt_out).build(TestCtx).unwrap();
    let registry = app.command_registry();
    registry.get("telemetry").is_some() || registry.group_metadata_for("telemetry").is_some()
}

async fn doctor_check_ids(h: &mut CliTestHarness<TestCtx>) -> Vec<String> {
    let out = h.run(&["demo", "doctor", "--json"]).await;
    let report: serde_json::Value = serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("doctor --json is not JSON: {e}\n{}", out.stdout));
    report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|f| f["check_id"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn an_opted_out_end_user_sees_no_first_run_notice() {
    let dir = tempfile::tempdir().unwrap();

    let mut control = harness(dir.path(), end_user(), false).with_interactive_stderr(true);
    let out = control.run(&["demo", "--help"]).await;
    assert!(
        out.stderr.contains(NOTICE_LINE),
        "control: an end user on a terminal sees the notice: {:?}",
        out.stderr
    );

    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path(), end_user(), true).with_interactive_stderr(true);
    let out = h.run(&["demo", "--help"]).await;
    assert!(
        !out.stderr.contains(NOTICE_LINE),
        "an app that collects nothing must not tell the person how to turn \
         collection on: {:?}",
        out.stderr
    );
}

#[test]
fn an_opted_out_end_user_has_no_telemetry_command_group() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        has_telemetry_group(dir.path(), end_user(), false),
        "control: an end-user app gets the telemetry group"
    );
    assert!(
        !has_telemetry_group(dir.path(), end_user(), true),
        "an opted-out end-user app has nothing to see or change"
    );
}

#[tokio::test]
async fn an_opted_out_end_user_has_no_telemetry_doctor_checks() {
    let dir = tempfile::tempdir().unwrap();
    let control = doctor_check_ids(&mut harness(dir.path(), end_user(), false)).await;
    assert!(
        control.iter().any(|id| id.starts_with("telemetry.")),
        "control: an end-user app gets the six telemetry checks: {control:?}"
    );

    let ids = doctor_check_ids(&mut harness(dir.path(), end_user(), true)).await;
    assert!(
        ids.iter().any(|id| id == "tmpdir-writable"),
        "the app's own checks still run: {ids:?}"
    );
    assert!(
        !ids.iter().any(|id| id.starts_with("telemetry.")),
        "no telemetry checks for a pipeline that never starts: {ids:?}"
    );
}

#[tokio::test]
async fn stored_consent_does_not_turn_an_opted_out_end_user_back_on() {
    // A person who said yes before the author opted out, or a settings file
    // copied from another app: the opt-out still wins, because startup --
    // the only step that reads the store -- never runs.
    let dir = tempfile::tempdir().unwrap();
    let mut before = harness(dir.path(), end_user(), false);
    let out = before.run(&["demo", "telemetry", "set", "usage"]).await;
    assert_eq!(out.exit_code, 0, "telemetry set failed: {}", out.stderr);

    let mut control = builder(dir.path(), end_user(), false)
        .build(TestCtx)
        .unwrap();
    control
        .run_with_args(vec!["demo".into(), "--version".into()])
        .await
        .unwrap();
    assert_eq!(
        control.telemetry_policy().level,
        TelemetryLevel::Usage,
        "control: stored consent turns an end-user app on"
    );

    let mut app = builder(dir.path(), end_user(), true)
        .build(TestCtx)
        .unwrap();
    app.run_with_args(vec!["demo".into(), "--version".into()])
        .await
        .unwrap();
    assert_eq!(app.telemetry_policy().level, TelemetryLevel::Off);
}

#[tokio::test]
async fn a_service_keeps_its_telemetry_under_the_opt_out() {
    let dir = tempfile::tempdir().unwrap();
    let ids = doctor_check_ids(&mut harness(dir.path(), Deployment::Service, true)).await;
    for id in [
        "telemetry.subscriber",
        "telemetry.store",
        "telemetry.endpoint",
        "telemetry.policy",
        "telemetry.identity",
        "telemetry.env",
    ] {
        assert!(
            ids.iter().any(|r| r == id),
            "a service's telemetry is the operator's decision, not the \
             opt-out's: {id} missing from {ids:?}"
        );
    }
    // A service never gets the group, opted out or not.
    assert!(!has_telemetry_group(dir.path(), Deployment::Service, true));
}
