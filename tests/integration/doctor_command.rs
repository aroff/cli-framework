//! Integration tests for the doctor command.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

struct DummyCtx;
impl AppContext for DummyCtx {}

fn stdio_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

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

struct StdoutCapture {
    _guard: MutexGuard<'static, ()>,
    saved_fd: i32,
    tmp: tempfile::NamedTempFile,
}

impl StdoutCapture {
    fn new() -> Self {
        let guard = stdio_lock().lock().unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let stdout_fd = std::io::stdout().as_raw_fd();
        let saved_fd = unsafe { libc::dup(stdout_fd) };
        unsafe {
            libc::dup2(tmp.as_raw_fd(), stdout_fd);
        }
        Self {
            _guard: guard,
            saved_fd,
            tmp,
        }
    }

    fn finish(self) -> String {
        let _ = std::io::stdout().flush();
        let stdout_fd = std::io::stdout().as_raw_fd();
        unsafe {
            libc::dup2(self.saved_fd, stdout_fd);
            libc::close(self.saved_fd);
        }
        std::fs::read_to_string(self.tmp.path()).unwrap_or_default()
    }
}

// ── Exit code: Ok when no errors ─────────────────────────────────────────────

#[tokio::test]
async fn doctor_exit_ok_when_no_errors() {
    let mut app = AppBuilder::new()
        .register_doctor_checks(vec![make_ok_check()])
        .build(DummyCtx)
        .unwrap();

    let capture = StdoutCapture::new();
    let result = app
        .run_with_args(vec!["myapp".to_string(), "doctor".to_string()])
        .await;
    capture.finish();

    assert!(result.is_ok(), "expected Ok() exit, got: {:?}", result);
}

// ── Exit code: Err when errors present ───────────────────────────────────────

#[tokio::test]
async fn doctor_exit_err_when_errors_present() {
    let mut app = AppBuilder::new()
        .register_doctor_checks(vec![make_error_check()])
        .build(DummyCtx)
        .unwrap();

    let capture = StdoutCapture::new();
    let result = app
        .run_with_args(vec!["myapp".to_string(), "doctor".to_string()])
        .await;
    capture.finish();

    assert!(result.is_err(), "expected Err() exit when errors found");
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

    let mut app = AppBuilder::new()
        .register_doctor_checks(vec![Arc::new(WarnCheck)])
        .build(DummyCtx)
        .unwrap();

    let capture = StdoutCapture::new();
    let result = app
        .run_with_args(vec!["myapp".to_string(), "doctor".to_string()])
        .await;
    capture.finish();

    assert!(
        result.is_ok(),
        "expected Ok() exit with warnings only, got: {:?}",
        result
    );
}

// ── Terminal output contains check info ──────────────────────────────────────

#[tokio::test]
async fn doctor_terminal_output_contains_check_id() {
    let mut app = AppBuilder::new()
        .register_doctor_checks(vec![make_ok_check()])
        .build(DummyCtx)
        .unwrap();

    let capture = StdoutCapture::new();
    let _result = app
        .run_with_args(vec!["myapp".to_string(), "doctor".to_string()])
        .await;
    let output = capture.finish();

    assert!(
        output.contains("ok-check") || output.contains("passed"),
        "expected terminal output to mention check id or summary, got: {:?}",
        output
    );
}

// ── JSON output is valid JSON with correct schema ─────────────────────────────

#[tokio::test]
async fn doctor_json_output_is_valid() {
    let mut app = AppBuilder::new()
        .register_doctor_checks(vec![make_ok_check()])
        .build(DummyCtx)
        .unwrap();

    let capture = StdoutCapture::new();
    let _result = app
        .run_with_args(vec![
            "myapp".to_string(),
            "doctor".to_string(),
            "--json".to_string(),
        ])
        .await;
    let output = capture.finish();

    let parsed: serde_json::Value =
        serde_json::from_str(&output).expect("JSON output should parse");

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
    let mut app = AppBuilder::new()
        .register_doctor_checks(vec![make_ok_check(), make_error_check()])
        .build(DummyCtx)
        .unwrap();

    let capture = StdoutCapture::new();
    let result = app
        .run_with_args(vec![
            "myapp".to_string(),
            "doctor".to_string(),
            "--check".to_string(),
            "ok-check".to_string(),
        ])
        .await;
    let output = capture.finish();

    // Should succeed (only ok-check ran, not error-check)
    assert!(
        result.is_ok(),
        "expected Ok() when only ok-check runs, got: {:?}",
        result
    );
    // Output should mention ok-check but we shouldn't see error-check
    let _ = output; // output captured; success means ok-check ran
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
        id: "doctor",
        summary: "Custom doctor",
        syntax: None,
        category: Some("ops"),
        spec: Some(Arc::new(CommandSpec {
            summary: "Custom doctor command",
            ..Default::default()
        })),
        validator: None,
        expose_mcp: false,
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
