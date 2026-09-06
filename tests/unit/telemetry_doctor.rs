// tests/unit/telemetry_doctor.rs
//! Unit tests for the six telemetry doctor checks (Task 25).
//!
//! Two fixups relative to the plan text:
//!
//! - `DoctorCheck::run` takes `&dyn AppContext` and returns a `DoctorFuture`
//!   (`Pin<Box<dyn Future<Output = DoctorFinding> + Send + 'static>>`), not a
//!   synchronous `fn run() -> DoctorFinding`. `run_all` is therefore `async`,
//!   awaits each check against a minimal no-op context (`TestCtx`, the same
//!   pattern already used by `tests/integration/telemetry_cli.rs` and
//!   throughout this crate's test suite), and every test that calls it is
//!   `#[tokio::test] async fn`.
//! - The plan's `an_unavailable_store_reports_the_reason_it_was_unavailable`
//!   and `every_finding_that_is_not_ok_carries_a_remediation` set
//!   `p.store = StoreState::Unavailable(...)` on a `TelemetryPolicy` — no
//!   such field exists (`TelemetryPolicy` carries `store_available: bool`
//!   and `store_error: Option<String>` instead). `StartupReport` (PR4,
//!   `src/telemetry/startup.rs`) has exactly the `pub store: StoreState`
//!   field the plan's test wants, and is already the type `StoreCheck`
//!   reads, so both tests set it there instead.

use cli_framework::app::AppContext;
use cli_framework::doctor::{CheckSeverity, DoctorFinding};
use cli_framework::telemetry::{
    telemetry_checks, Deployment, StartupReport, StoreState, SubscriberOutcome, TelemetryLevel,
};
use std::sync::Arc;

mod support;
use support::policy_with;

struct TestCtx;
impl AppContext for TestCtx {}

async fn run_all(
    policy: cli_framework::telemetry::TelemetryPolicy,
    report: StartupReport,
) -> Vec<DoctorFinding> {
    let ctx = TestCtx;
    let mut findings = Vec::new();
    for check in telemetry_checks(Arc::new(policy), Arc::new(report)) {
        findings.push(check.run(&ctx).await);
    }
    findings
}

fn finding<'a>(findings: &'a [DoctorFinding], id: &str) -> &'a DoctorFinding {
    findings
        .iter()
        .find(|f| f.check_id == id)
        .unwrap_or_else(|| panic!("{id} missing"))
}

#[tokio::test]
async fn all_six_checks_are_present_with_the_documented_ids() {
    let findings = run_all(
        policy_with(
            Deployment::EndUser { privacy_url: None },
            TelemetryLevel::Usage,
            |_| {},
        ),
        StartupReport::default(),
    )
    .await;
    let mut ids: Vec<&str> = findings.iter().map(|f| f.check_id.as_str()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "telemetry.endpoint",
            "telemetry.env",
            "telemetry.identity",
            "telemetry.policy",
            "telemetry.store",
            "telemetry.subscriber",
        ]
    );
}

#[tokio::test]
async fn a_foreign_subscriber_is_a_warning_not_an_error() {
    let report = StartupReport {
        subscriber: SubscriberOutcome::ForeignSubscriber,
        ..Default::default()
    };
    let findings = run_all(
        policy_with(
            Deployment::EndUser { privacy_url: None },
            TelemetryLevel::Usage,
            |_| {},
        ),
        report,
    )
    .await;
    let f = finding(&findings, "telemetry.subscriber");
    assert_eq!(
        f.severity,
        CheckSeverity::Warning,
        "another crate owning the subscriber is a normal, working configuration \
         with reduced telemetry, not a broken install"
    );
    assert!(
        f.message.contains("metrics"),
        "and it says what still works: {}",
        f.message
    );
}

#[tokio::test]
async fn an_unavailable_store_reports_the_reason_it_was_unavailable() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Off,
        |_| {},
    );
    let report = StartupReport {
        store: StoreState::Unavailable("permission denied on /home/x/.config".into()),
        ..Default::default()
    };
    let findings = run_all(policy, report).await;
    let store = finding(&findings, "telemetry.store");
    assert_eq!(store.severity, CheckSeverity::Warning);
    assert!(
        store.message.contains("permission denied"),
        "{}",
        store.message
    );
}

#[tokio::test]
async fn no_configured_endpoint_is_skipped_rather_than_ok() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Off,
        |p| {
            p.endpoint = None;
        },
    );
    let findings = run_all(policy, StartupReport::default()).await;
    assert_eq!(
        finding(&findings, "telemetry.endpoint").severity,
        CheckSeverity::Skipped,
        "'nothing to check' and 'checked and fine' are different states"
    );
}

#[tokio::test]
async fn no_policy_client_is_skipped_and_says_not_managed() {
    let findings = run_all(
        policy_with(
            Deployment::EndUser { privacy_url: None },
            TelemetryLevel::Usage,
            |_| {},
        ),
        StartupReport::default(),
    )
    .await;
    let f = finding(&findings, "telemetry.policy");
    assert_eq!(f.severity, CheckSeverity::Skipped);
    assert!(f.message.contains("not managed"), "{}", f.message);
}

#[tokio::test]
async fn an_anonymous_install_reports_identity_ok_with_no_identifier() {
    use cli_framework::telemetry::Attribution;
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.attribution = Attribution::Anonymous;
            p.install_id = None;
        },
    );
    let findings = run_all(policy, StartupReport::default()).await;
    let f = finding(&findings, "telemetry.identity");
    assert_eq!(
        f.severity,
        CheckSeverity::Ok,
        "anonymous is a valid configuration, not a fault"
    );
    assert!(
        !f.message.contains("install-"),
        "and the check does not print the identifier it says is absent"
    );
}

#[tokio::test]
async fn an_unmatched_telemetry_environment_variable_is_reported_with_its_name() {
    let report = StartupReport {
        unmatched_env: vec!["DEMO_TELEMETRY_LEVELS".to_string()],
        ..Default::default()
    };
    let findings = run_all(
        policy_with(
            Deployment::EndUser { privacy_url: None },
            TelemetryLevel::Usage,
            |_| {},
        ),
        report,
    )
    .await;
    let f = finding(&findings, "telemetry.env");
    assert_eq!(f.severity, CheckSeverity::Warning);
    assert!(
        f.message.contains("DEMO_TELEMETRY_LEVELS"),
        "a typo'd variable silently doing nothing is the whole reason this \
         check exists: {}",
        f.message
    );
}

#[tokio::test]
async fn no_unmatched_variables_is_ok_not_skipped() {
    let findings = run_all(
        policy_with(
            Deployment::EndUser { privacy_url: None },
            TelemetryLevel::Usage,
            |_| {},
        ),
        StartupReport::default(),
    )
    .await;
    assert_eq!(
        finding(&findings, "telemetry.env").severity,
        CheckSeverity::Ok
    );
}

#[tokio::test]
async fn no_check_message_contains_the_install_identifier() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Debug,
        |p| {
            p.install_id = Some("11111111-2222-3333-4444-555555555555".into());
        },
    );
    for f in run_all(policy, StartupReport::default()).await {
        assert!(
            !f.message.contains("11111111-2222"),
            "doctor output gets pasted into bug reports: {} / {}",
            f.check_id,
            f.message
        );
        if let Some(detail) = &f.detail {
            assert!(
                !detail.contains("11111111-2222"),
                "{}: {detail}",
                f.check_id
            );
        }
    }
}

#[tokio::test]
async fn every_finding_that_is_not_ok_carries_a_remediation() {
    let report = StartupReport {
        subscriber: SubscriberOutcome::ForeignSubscriber,
        unmatched_env: vec!["DEMO_TELEMETRY_X".into()],
        store: StoreState::Unavailable("no config dir".into()),
        ..Default::default()
    };
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |_| {},
    );
    for f in run_all(policy, report).await {
        if f.severity == CheckSeverity::Warning || f.severity == CheckSeverity::Error {
            assert!(
                f.remediation.is_some(),
                "{} tells a person something is wrong without telling them what to do",
                f.check_id
            );
        }
    }
}

#[tokio::test]
async fn every_check_exposes_its_documented_id_title_and_description() {
    let policy = Arc::new(policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |_| {},
    ));
    let report = Arc::new(StartupReport::default());
    let checks = telemetry_checks(policy, report);
    // Fixed order from `telemetry_checks`'s own `vec![...]`; a doctor UI or a
    // support playbook can reasonably index into this list, so a silent
    // reorder is as much a regression as a renamed id would be.
    let expected: &[(&str, &str, &str)] = &[
        ("telemetry.subscriber", "Tracing subscriber", "installed"),
        ("telemetry.store", "Settings store", "settings file"),
        (
            "telemetry.endpoint",
            "Collector reachability",
            "OTLP collector",
        ),
        ("telemetry.policy", "Managed policy", "policy client"),
        ("telemetry.identity", "Attribution", "attribution mode"),
        ("telemetry.env", "Environment variables", "TELEMETRY_*"),
    ];
    assert_eq!(checks.len(), expected.len());
    for (check, (id, title, description_fragment)) in checks.iter().zip(expected) {
        assert_eq!(check.id(), *id);
        assert_eq!(check.title(), *title, "{id}");
        let description = check
            .description()
            .unwrap_or_else(|| panic!("{id} has no description"));
        assert!(
            description.contains(description_fragment),
            "{id}: {description}"
        );
    }
}

#[tokio::test]
async fn an_endpoint_with_no_parseable_host_and_port_is_a_warning() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            // No "://" and an empty host before the last ':' — `host_port`
            // rejects this before any socket work is attempted.
            p.endpoint = Some(":4318".to_string());
        },
    );
    let findings = run_all(policy, StartupReport::default()).await;
    let f = finding(&findings, "telemetry.endpoint");
    assert_eq!(f.severity, CheckSeverity::Warning);
    assert!(
        f.message.contains("no parseable host:port"),
        "{}",
        f.message
    );
    assert!(f.remediation.is_some());
}

#[tokio::test]
async fn a_ready_store_reports_ok_with_its_path() {
    let report = StartupReport {
        store: StoreState::Ready(std::path::PathBuf::from(
            "/home/x/.config/app/telemetry.json",
        )),
        ..Default::default()
    };
    let findings = run_all(
        policy_with(
            Deployment::EndUser { privacy_url: None },
            TelemetryLevel::Usage,
            |_| {},
        ),
        report,
    )
    .await;
    let f = finding(&findings, "telemetry.store");
    assert_eq!(f.severity, CheckSeverity::Ok);
    assert!(
        f.message.contains("/home/x/.config/app/telemetry.json"),
        "{}",
        f.message
    );
    assert!(f.remediation.is_none());
}

#[tokio::test]
async fn an_identified_install_names_the_hook_not_a_principal_value() {
    use cli_framework::telemetry::Attribution;
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.attribution = Attribution::Identified;
        },
    );
    let findings = run_all(policy, StartupReport::default()).await;
    let f = finding(&findings, "telemetry.identity");
    assert_eq!(f.severity, CheckSeverity::Ok);
    assert!(f.message.contains("identified"), "{}", f.message);
}

#[tokio::test]
async fn a_reachable_collector_reports_ok() {
    // A bound-but-not-yet-`accept`ed loopback listener still completes a
    // client-side TCP handshake (the kernel queues it), so this is a real,
    // deterministic "reachable" outcome with no external network dependency.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("local_addr").port();
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.endpoint = Some(format!("http://127.0.0.1:{port}"));
        },
    );
    let findings = run_all(policy, StartupReport::default()).await;
    let f = finding(&findings, "telemetry.endpoint");
    assert_eq!(f.severity, CheckSeverity::Ok, "{}", f.message);
    assert!(f.message.contains("reached the configured collector"));
    assert!(f.remediation.is_none());
    drop(listener);
}
