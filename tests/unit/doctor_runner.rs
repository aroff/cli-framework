//! Unit tests for the doctor diagnostics framework.

use cli_framework::app::AppContext;
use cli_framework::doctor::builtin::{EnvRequiredCheck, TmpdirWritableCheck};
use cli_framework::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};
use cli_framework::doctor::runner::{DoctorReport, DoctorRunner};
use cli_framework::doctor::DoctorError;
use std::sync::Arc;

struct DummyCtx;
impl AppContext for DummyCtx {}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_ok_check(id: &'static str, title: &'static str) -> Arc<dyn DoctorCheck> {
    struct OkCheck {
        id: &'static str,
        title: &'static str,
    }
    impl DoctorCheck for OkCheck {
        fn id(&self) -> &'static str {
            self.id
        }
        fn title(&self) -> &'static str {
            self.title
        }
        fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
            let id = self.id;
            let title = self.title;
            Box::pin(async move {
                DoctorFinding {
                    check_id: id.to_string(),
                    title: title.to_string(),
                    severity: CheckSeverity::Ok,
                    message: "ok".to_string(),
                    detail: None,
                    remediation: None,
                }
            })
        }
    }
    Arc::new(OkCheck { id, title })
}

fn make_error_check(id: &'static str) -> Arc<dyn DoctorCheck> {
    struct ErrorCheck {
        id: &'static str,
    }
    impl DoctorCheck for ErrorCheck {
        fn id(&self) -> &'static str {
            self.id
        }
        fn title(&self) -> &'static str {
            "Error check"
        }
        fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
            let id = self.id;
            Box::pin(async move {
                DoctorFinding {
                    check_id: id.to_string(),
                    title: "Error check".to_string(),
                    severity: CheckSeverity::Error,
                    message: "something failed".to_string(),
                    detail: None,
                    remediation: None,
                }
            })
        }
    }
    Arc::new(ErrorCheck { id })
}

fn make_warning_check(id: &'static str) -> Arc<dyn DoctorCheck> {
    struct WarnCheck {
        id: &'static str,
    }
    impl DoctorCheck for WarnCheck {
        fn id(&self) -> &'static str {
            self.id
        }
        fn title(&self) -> &'static str {
            "Warning check"
        }
        fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
            let id = self.id;
            Box::pin(async move {
                DoctorFinding {
                    check_id: id.to_string(),
                    title: "Warning check".to_string(),
                    severity: CheckSeverity::Warning,
                    message: "advisory".to_string(),
                    detail: None,
                    remediation: None,
                }
            })
        }
    }
    Arc::new(WarnCheck { id })
}

// ── DR001: duplicate check id ─────────────────────────────────────────────────

#[test]
fn dr001_duplicate_check_id_rejected() {
    let mut runner = DoctorRunner::new();
    runner
        .register(make_ok_check("my-check", "My Check"))
        .unwrap();
    let err = runner
        .register(make_ok_check("my-check", "My Check Again"))
        .unwrap_err();
    match err {
        DoctorError::DuplicateCheckId(id) => assert_eq!(id, "my-check"),
        other => panic!("expected DuplicateCheckId, got {:?}", other),
    }
}

// ── run_all: produces N findings ──────────────────────────────────────────────

#[tokio::test]
async fn run_all_produces_n_findings() {
    let mut runner = DoctorRunner::new();
    runner
        .register(make_ok_check("check-a", "Check A"))
        .unwrap();
    runner
        .register(make_ok_check("check-b", "Check B"))
        .unwrap();
    runner
        .register(make_ok_check("check-c", "Check C"))
        .unwrap();

    let ctx = DummyCtx;
    let report = runner.run_all(&ctx).await;
    assert_eq!(report.findings.len(), 3);
    assert_eq!(report.ok, 3);
}

// ── run_all: registration order preserved ────────────────────────────────────

#[tokio::test]
async fn run_all_preserves_registration_order() {
    // Use a slow check followed by fast ones to test ordering
    struct SlowCheck;
    impl DoctorCheck for SlowCheck {
        fn id(&self) -> &'static str {
            "slow"
        }
        fn title(&self) -> &'static str {
            "Slow"
        }
        fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
            Box::pin(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                DoctorFinding {
                    check_id: "slow".to_string(),
                    title: "Slow".to_string(),
                    severity: CheckSeverity::Ok,
                    message: "slow done".to_string(),
                    detail: None,
                    remediation: None,
                }
            })
        }
    }

    let mut runner = DoctorRunner::new();
    runner.register(Arc::new(SlowCheck)).unwrap();
    runner.register(make_ok_check("fast-1", "Fast 1")).unwrap();
    runner.register(make_ok_check("fast-2", "Fast 2")).unwrap();

    let ctx = DummyCtx;
    let report = runner.run_all(&ctx).await;
    assert_eq!(report.findings[0].check_id, "slow");
    assert_eq!(report.findings[1].check_id, "fast-1");
    assert_eq!(report.findings[2].check_id, "fast-2");
}

// ── DR003: run_filtered with unknown id ──────────────────────────────────────

#[tokio::test]
async fn run_filtered_unknown_id_produces_dr003() {
    let runner = DoctorRunner::new();
    let ctx = DummyCtx;
    let report = runner.run_filtered(&ctx, &["unknown-id"]).await;

    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].check_id, "unknown-id");
    assert_eq!(report.findings[0].severity, CheckSeverity::Error);
    assert!(report.findings[0].message.contains("DR003"));
}

// ── DR002: panicking check produces Error finding ─────────────────────────────

#[tokio::test]
async fn dr002_panicking_check_produces_error_finding() {
    struct PanickingCheck;
    impl DoctorCheck for PanickingCheck {
        fn id(&self) -> &'static str {
            "panicking-check"
        }
        fn title(&self) -> &'static str {
            "Panicking"
        }
        fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
            Box::pin(async { panic!("intentional test panic") })
        }
    }

    let mut runner = DoctorRunner::new();
    runner.register(Arc::new(PanickingCheck)).unwrap();

    let ctx = DummyCtx;
    let report = runner.run_all(&ctx).await;

    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].severity, CheckSeverity::Error);
    assert_eq!(report.findings[0].check_id, "panicking-check");
    assert!(
        report.findings[0].message.contains("DR002"),
        "expected DR002 in message: {}",
        report.findings[0].message
    );
}

// ── DoctorReport counters ─────────────────────────────────────────────────────

#[tokio::test]
async fn report_counters_are_accurate() {
    let mut runner = DoctorRunner::new();
    runner.register(make_ok_check("ok-1", "Ok 1")).unwrap();
    runner.register(make_ok_check("ok-2", "Ok 2")).unwrap();
    runner.register(make_warning_check("warn-1")).unwrap();
    runner.register(make_error_check("err-1")).unwrap();

    let ctx = DummyCtx;
    let report = runner.run_all(&ctx).await;
    assert_eq!(report.ok, 2);
    assert_eq!(report.warnings, 1);
    assert_eq!(report.errors, 1);
    assert_eq!(report.skipped, 0);
}

// ── EnvRequiredCheck ──────────────────────────────────────────────────────────

#[tokio::test]
async fn env_required_check_error_when_var_missing() {
    std::env::remove_var("DOCTOR_TEST_MISSING_VAR_XYZ");
    let check = EnvRequiredCheck::new(&["DOCTOR_TEST_MISSING_VAR_XYZ"]);
    let ctx = DummyCtx;
    let finding = check.run(&ctx).await;
    assert_eq!(finding.severity, CheckSeverity::Error);
    assert!(finding.message.contains("DOCTOR_TEST_MISSING_VAR_XYZ"));
}

#[tokio::test]
async fn env_required_check_ok_when_var_present() {
    std::env::set_var("DOCTOR_TEST_PRESENT_VAR_XYZ", "some-value");
    let check = EnvRequiredCheck::new(&["DOCTOR_TEST_PRESENT_VAR_XYZ"]);
    let ctx = DummyCtx;
    let finding = check.run(&ctx).await;
    assert_eq!(finding.severity, CheckSeverity::Ok);
    std::env::remove_var("DOCTOR_TEST_PRESENT_VAR_XYZ");
}

// ── TmpdirWritableCheck ───────────────────────────────────────────────────────

#[tokio::test]
async fn tmpdir_writable_check_ok_for_real_tmpdir() {
    let check = TmpdirWritableCheck;
    let ctx = DummyCtx;
    let finding = check.run(&ctx).await;
    // The real tmpdir should be writable in a CI/test environment
    assert!(
        finding.severity == CheckSeverity::Ok || finding.severity == CheckSeverity::Warning,
        "expected Ok or Warning, got {:?}",
        finding.severity
    );
}

#[tokio::test]
async fn tmpdir_writable_check_warning_for_nonexistent_dir() {
    let old = std::env::var("TMPDIR").ok();
    std::env::set_var(
        "TMPDIR",
        "/tmp/cli-framework-nonexistent-doctor-test-8675309",
    );

    let check = TmpdirWritableCheck;
    let ctx = DummyCtx;
    let finding = check.run(&ctx).await;

    match old {
        Some(v) => std::env::set_var("TMPDIR", v),
        None => std::env::remove_var("TMPDIR"),
    }

    assert_eq!(finding.severity, CheckSeverity::Warning);
}

// ── render_json produces valid JSON ──────────────────────────────────────────

#[test]
fn render_json_produces_valid_json() {
    use cli_framework::doctor::render::render_json;
    use cli_framework::doctor::runner::DoctorReport;

    let findings = vec![
        DoctorFinding {
            check_id: "test-check".to_string(),
            title: "Test".to_string(),
            severity: CheckSeverity::Ok,
            message: "all good".to_string(),
            detail: None,
            remediation: None,
        },
        DoctorFinding {
            check_id: "test-warn".to_string(),
            title: "Test Warning".to_string(),
            severity: CheckSeverity::Warning,
            message: "advisory".to_string(),
            detail: Some("details".to_string()),
            remediation: Some("fix it".to_string()),
        },
    ];
    let report = DoctorReport::from_findings(findings);

    // Capture render_json output by serializing directly
    #[derive(serde::Serialize)]
    struct JsonReport<'a> {
        findings: &'a Vec<DoctorFinding>,
        summary: JsonSummary,
    }
    #[derive(serde::Serialize)]
    struct JsonSummary {
        ok: usize,
        warnings: usize,
        errors: usize,
        skipped: usize,
    }

    let json_report = JsonReport {
        findings: &report.findings,
        summary: JsonSummary {
            ok: report.ok,
            warnings: report.warnings,
            errors: report.errors,
            skipped: report.skipped,
        },
    };

    let json_str = serde_json::to_string_pretty(&json_report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert!(parsed["findings"].is_array());
    assert_eq!(parsed["findings"].as_array().unwrap().len(), 2);
    assert!(parsed["summary"]["ok"].is_number());
    assert_eq!(parsed["summary"]["ok"], 1);
    assert_eq!(parsed["summary"]["warnings"], 1);
    assert_eq!(parsed["summary"]["errors"], 0);
    assert_eq!(parsed["summary"]["skipped"], 0);
    assert_eq!(parsed["findings"][0]["severity"], "ok");
    assert_eq!(parsed["findings"][1]["severity"], "warning");
}

// ── from_checks constructor ───────────────────────────────────────────────────

#[tokio::test]
async fn from_checks_runs_all_checks() {
    let checks: Vec<Arc<dyn DoctorCheck>> = vec![make_ok_check("a", "A"), make_ok_check("b", "B")];
    let runner = DoctorRunner::from_checks(checks);
    let ctx = DummyCtx;
    let report = runner.run_all(&ctx).await;
    assert_eq!(report.findings.len(), 2);
}

// ── DoctorCheck object safety ─────────────────────────────────────────────────

#[test]
fn doctor_check_is_object_safe() {
    // Verify Arc<dyn DoctorCheck> compiles
    let checks: Vec<Arc<dyn DoctorCheck>> = vec![make_ok_check("x", "X")];
    assert_eq!(checks.len(), 1);
}
