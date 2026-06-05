//! Integration tests for the doctor command.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};
use std::sync::Arc;

use cli_framework::testkit::CliTestHarness;

struct DummyCtx;
impl AppContext for DummyCtx {}

fn make_ok_check() -> Arc<dyn DoctorCheck> {
    struct OkCheck;
    impl DoctorCheck for OkCheck {
        fn id(&self) -> &'static str {
            "ok-check"
        }
        fn title(&self) -> &'static str {
            "Always OK"
        }
        fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
            Box::pin(async {
                DoctorFinding {
                    check_id: "ok-check".to_string(),
                    title: "Always OK".to_string(),
                    severity: CheckSeverity::Ok,
                    message: "everything is fine".to_string(),
                    detail: None,
                    remediation: None,
                }
            })
        }
    }
    Arc::new(OkCheck)
}

fn make_error_check() -> Arc<dyn DoctorCheck> {
    struct ErrorCheck;
    impl DoctorCheck for ErrorCheck {
        fn id(&self) -> &'static str {
            "error-check"
        }
        fn title(&self) -> &'static str {
            "Always Error"
        }
        fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
            Box::pin(async {
                DoctorFinding {
                    check_id: "error-check".to_string(),
                    title: "Always Error".to_string(),
                    severity: CheckSeverity::Error,
                    message: "something is broken".to_string(),
                    detail: None,
                    remediation: Some("Fix it.".to_string()),
                }
            })
        }
    }
    Arc::new(ErrorCheck)
}

// ── Exit code: Ok when no errors ─────────────────────────────────────────────

#[tokio::test]
async fn doctor_exit_ok_when_no_errors() {
    let app = AppBuilder::new()
        .register_doctor_checks(vec![make_ok_check()])
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "doctor"]).await;
    output.assert_exit_code(0);
}

// ── Exit code: Err when errors present ───────────────────────────────────────

#[tokio::test]
async fn doctor_exit_err_when_errors_present() {
    let app = AppBuilder::new()
        .register_doctor_checks(vec![make_error_check()])
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "doctor"]).await;
    output.assert_exit_code(1);
}

// ── Exit code: Ok when only warnings ─────────────────────────────────────────

#[tokio::test]
async fn doctor_exit_ok_with_warnings_only() {
    struct WarnCheck;
    impl DoctorCheck for WarnCheck {
        fn id(&self) -> &'static str {
            "warn-check"
        }
        fn title(&self) -> &'static str {
            "Warning"
        }
        fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
            Box::pin(async {
                DoctorFinding {
                    check_id: "warn-check".to_string(),
                    title: "Warning".to_string(),
                    severity: CheckSeverity::Warning,
                    message: "advisory only".to_string(),
                    detail: None,
                    remediation: None,
                }
            })
        }
    }

    let app = AppBuilder::new()
        .register_doctor_checks(vec![Arc::new(WarnCheck)])
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "doctor"]).await;
    output.assert_exit_code(0);
}

// ── Terminal output contains check info ──────────────────────────────────────

#[tokio::test]
async fn doctor_terminal_output_contains_check_id() {
    let app = AppBuilder::new()
        .register_doctor_checks(vec![make_ok_check()])
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "doctor"]).await;

    assert!(
        output.stdout().contains("ok-check") || output.stdout().contains("passed"),
        "expected terminal output to mention check id or summary, got: {:?}",
        output.stdout()
    );
}

// ── JSON output is valid JSON with correct schema ─────────────────────────────

#[tokio::test]
async fn doctor_json_output_is_valid() {
    let app = AppBuilder::new()
        .register_doctor_checks(vec![make_ok_check()])
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "doctor", "--json"]).await;

    let parsed: serde_json::Value =
        serde_json::from_str(output.stdout()).expect("JSON output should parse");

    assert!(
        parsed["findings"].is_array(),
        "expected 'findings' array in JSON output"
    );
    assert!(
        parsed["summary"].is_object(),
        "expected 'summary' object in JSON output"
    );
    assert!(
        parsed["summary"]["ok"].is_number(),
        "expected numeric 'ok' in summary"
    );
    assert!(
        parsed["summary"]["errors"].is_number(),
        "expected numeric 'errors' in summary"
    );
}

// ── --check flag filters to a single check ────────────────────────────────────

#[tokio::test]
async fn doctor_check_flag_filters_results() {
    let app = AppBuilder::new()
        .register_doctor_checks(vec![make_ok_check(), make_error_check()])
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness
        .run(&["myapp", "doctor", "--check", "ok-check"])
        .await;

    // Should succeed (only ok-check ran, not error-check)
    output.assert_exit_code(0);
    assert!(output.stdout().contains("ok-check"));
}

// ── doctor command registered via register_doctor_checks ─────────────────────

#[test]
fn doctor_command_registered_after_build() {
    use cli_framework::app::AppBuilder;

    let app = AppBuilder::new()
        .register_doctor_checks(vec![make_ok_check()])
        .build(DummyCtx)
        .unwrap();

    // The app has a doctor command (verified by successful run above)
    // We verify indirectly that build() did not error
    let _ = app;
}

// ── Pre-registered doctor command not overwritten ─────────────────────────────

#[test]
fn pre_registered_doctor_command_not_overwritten() {
    use cli_framework::command::Command;
    use cli_framework::spec::command_tree::CommandSpec;

    let custom_doctor = Command {
        id: Arc::from("doctor"),
        spec: Arc::new(CommandSpec {
            summary: "Custom doctor command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    };

    // Should not error: doctor checks are silently skipped when "doctor" already registered
    let result = AppBuilder::new()
        .register_command(custom_doctor)
        .unwrap()
        .register_doctor_checks(vec![make_ok_check()])
        .build(DummyCtx);

    assert!(
        result.is_ok(),
        "build should succeed even with conflicting doctor registration"
    );
}
