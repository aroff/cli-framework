// tests/unit/telemetry_doctor_wiring.rs
//! The deferred handle that lets the six telemetry doctor checks be
//! registered at *build* time and still answer about *run* time
//! (`specs/029-telemetry-deferred-wiring.md`, deferral 1).
//!
//! Two clocks have to be reconciled. `AppBuilder::push_doctor_checks` is the
//! only door into the check list, and `build()` consumes the builder and
//! moves that list into the `doctor` command, so registration cannot happen
//! any later than `build`. But both values the checks read — the
//! `Arc<TelemetryPolicy>` and the `Arc<StartupReport>` — are produced by
//! `run_startup`, which runs on the first dispatch, long after `build` has
//! returned. A check handed the build-time policy would answer about a
//! resolution taken before the settings store was opened: it would report the
//! level and attribution the person would have had if they had never run
//! `telemetry set`, which is the one answer nobody running `doctor` wants.
//!
//! `StartupCell<T>` is the reconciliation, and this file is its contract:
//! the semantics on their own (part 1), the honest `Skipped` an unfilled cell
//! produces (part 2), the late write actually reaching a check that was
//! registered before the value existed (part 3), and the whole seam observed
//! from outside through a real `doctor` invocation on a real app (part 4).
//!
//! Part 4 also pins a behaviour change this wiring introduced. Because the
//! framework now registers six checks for every telemetry-enabled app,
//! `doctor_checks` is never empty, so `build()`'s auto-registration gives
//! every such app a `doctor` command it did not ask for — deliberately, the
//! same way it already gets a `telemetry` command group.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::doctor::{CheckSeverity, DoctorFinding};
use cli_framework::telemetry::{
    telemetry_checks, Deployment, StartupCell, StartupReport, StoreState, SubscriberOutcome,
    TelemetryLevel, TelemetryPolicy,
};
use cli_framework::testkit::CliTestHarness;
use std::path::Path;
use std::sync::Arc;

mod support;
use support::policy_with;

struct TestCtx;
impl AppContext for TestCtx {}

/// The message [`awaiting_startup`](cli_framework::telemetry) puts on a
/// finding whose cell is still empty. Asserting on the sentence rather than
/// on `CheckSeverity::Skipped` alone is what keeps these tests falsifiable:
/// four of the six checks have their *own* legitimate reasons to skip (no
/// endpoint configured, no policy client, anonymous attribution), so
/// "severity is Skipped" would pass with the cell wiring deleted.
const AWAITING: &str = "telemetry startup has not run in this process";

fn a_policy() -> TelemetryPolicy {
    policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |_| {},
    )
}

fn a_report() -> StartupReport {
    StartupReport {
        subscriber: SubscriberOutcome::Installed,
        store: StoreState::Ready(std::path::PathBuf::from("/tmp/demo/telemetry.json")),
        ..Default::default()
    }
}

async fn run_all(
    checks: &[Arc<dyn cli_framework::doctor::check::DoctorCheck>],
) -> Vec<DoctorFinding> {
    let ctx = TestCtx;
    let mut findings = Vec::new();
    for check in checks {
        findings.push(check.run(&ctx).await);
    }
    findings
}

fn finding<'a>(findings: &'a [DoctorFinding], id: &str) -> &'a DoctorFinding {
    findings
        .iter()
        .find(|f| f.check_id == id)
        .unwrap_or_else(|| panic!("{id} missing from {:?}", ids(findings)))
}

fn ids(findings: &[DoctorFinding]) -> Vec<&str> {
    findings.iter().map(|f| f.check_id.as_str()).collect()
}

// ── part 1: the cell on its own ──────────────────────────────────────────

#[test]
fn an_empty_cell_answers_none_until_something_fills_it() {
    let cell: StartupCell<TelemetryPolicy> = StartupCell::empty();
    assert!(
        cell.get().is_none(),
        "a cell nobody filled must answer None"
    );

    cell.set(Arc::new(a_policy()));
    assert!(
        cell.get().is_some(),
        "the value must be visible through the same handle that was empty"
    );
}

#[test]
fn a_filled_cell_answers_immediately() {
    let cell = StartupCell::filled(Arc::new(a_policy()));
    assert_eq!(
        cell.get().expect("filled").level,
        TelemetryLevel::Usage,
        "a seeded cell must answer without waiting for a set()"
    );
}

#[test]
fn set_overwrites_rather_than_refusing() {
    // The behaviour that rules out `OnceLock`. `build` seeds the policy cell
    // with the pre-consent resolution so an app that is built and never run
    // still answers; startup then writes the resolution that actually saw the
    // settings store. If the first write won, the seed would permanently
    // shadow the real answer — the exact bug the cell exists to avoid.
    let cell = StartupCell::filled(Arc::new(policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Off,
        |_| {},
    )));

    cell.set(Arc::new(policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Diagnostic,
        |_| {},
    )));

    assert_eq!(
        cell.get().expect("filled").level,
        TelemetryLevel::Diagnostic,
        "the later write — startup's — must win over the build-time seed"
    );
}

#[test]
fn a_clone_shares_the_slot_so_a_late_write_reaches_an_early_reader() {
    // `App` keeps one handle and each of the six checks keeps another. They
    // are only useful if they are the same slot: `Clone` here must share, not
    // snapshot.
    let held_by_the_app = StartupCell::<StartupReport>::empty();
    let handed_to_a_check = held_by_the_app.clone();

    assert!(handed_to_a_check.get().is_none());
    held_by_the_app.set(Arc::new(a_report()));

    assert!(
        handed_to_a_check.get().is_some(),
        "a clone taken before the write must observe it; cloning the slot, \
         not the contents, is what makes the deferral work"
    );
}

#[test]
fn a_finished_arc_converts_into_a_filled_cell() {
    // `telemetry_checks` takes `impl Into<StartupCell<_>>` precisely so that
    // a caller who already holds the value — every test that builds a policy
    // and a report by hand — passes it with no ceremony.
    let cell: StartupCell<TelemetryPolicy> = Arc::new(a_policy()).into();
    assert!(cell.get().is_some());
}

#[test]
fn the_default_cell_is_empty() {
    assert!(StartupCell::<StartupReport>::default().get().is_none());
}

#[test]
fn debug_reports_whether_the_cell_is_filled_and_never_its_contents() {
    // `App` derives nothing, but `TelemetryPolicy` is `Debug` and carries the
    // per-install id. A cell that printed its payload would put that id into
    // any log line that debug-formatted a struct holding one.
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| p.install_id = Some("11111111-2222-3333-4444-555555555555".to_string()),
    );
    let rendered = format!("{:?}", StartupCell::filled(Arc::new(policy)));

    assert!(
        rendered.contains("filled: true"),
        "expected the filled flag, got {rendered}"
    );
    assert!(
        !rendered.contains("11111111-2222-3333-4444-555555555555"),
        "the install id leaked into a Debug rendering: {rendered}"
    );
}

// ── part 2: an unfilled cell is Skipped, not Ok ──────────────────────────

#[tokio::test]
async fn every_cell_backed_check_reports_skipped_while_its_cell_is_empty() {
    let checks = telemetry_checks(
        StartupCell::<TelemetryPolicy>::empty(),
        StartupCell::<StartupReport>::empty(),
    );
    let findings = run_all(&checks).await;

    for id in [
        "telemetry.subscriber",
        "telemetry.store",
        "telemetry.endpoint",
        "telemetry.identity",
        "telemetry.env",
    ] {
        let f = finding(&findings, id);
        assert_eq!(
            f.severity,
            CheckSeverity::Skipped,
            "{id} must not claim Ok about a startup that never ran"
        );
        assert!(
            f.message.contains(AWAITING),
            "{id} skipped for the wrong reason: {}",
            f.message
        );
    }
}

#[tokio::test]
async fn a_check_with_an_empty_cell_still_reports_its_own_id_and_title() {
    // A finding is useless if the doctor cannot say which check produced it,
    // and the empty-cell path builds its finding by hand rather than falling
    // through the check's normal construction — so the two could drift.
    let checks = telemetry_checks(
        StartupCell::<TelemetryPolicy>::empty(),
        StartupCell::<StartupReport>::empty(),
    );
    let findings = run_all(&checks).await;

    for check in &checks {
        let f = finding(&findings, check.id());
        assert_eq!(f.check_id, check.id());
        assert_eq!(
            f.title,
            check.title(),
            "{} reported a title that does not match its own",
            check.id()
        );
    }
}

#[tokio::test]
async fn the_policy_check_skips_for_its_own_reason_not_for_a_missing_cell() {
    // `PolicyCheck` is a unit struct — it reads no cell at all. If it ever
    // started reporting the awaiting-startup message, that would mean the
    // empty-cell path had been wired into a check that has nothing to wait
    // for.
    let checks = telemetry_checks(
        StartupCell::<TelemetryPolicy>::empty(),
        StartupCell::<StartupReport>::empty(),
    );
    let findings = run_all(&checks).await;
    let f = finding(&findings, "telemetry.policy");

    assert_eq!(f.severity, CheckSeverity::Skipped);
    assert!(
        !f.message.contains(AWAITING),
        "telemetry.policy has no cell to wait for: {}",
        f.message
    );
}

// ── part 3: the late write is what the check reads ───────────────────────

#[tokio::test]
async fn a_check_registered_before_the_value_existed_reads_it_once_startup_writes_it() {
    // The whole point of the design, in one test: build the checks against
    // empty cells (what `AppBuilder::build` does for the report), run them,
    // fill the cells (what `App::init_telemetry` does), run the same checks
    // again, and watch the answer change. Nothing re-registers in between.
    let policy_cell = StartupCell::<TelemetryPolicy>::empty();
    let report_cell = StartupCell::<StartupReport>::empty();
    let checks = telemetry_checks(policy_cell.clone(), report_cell.clone());

    let before = run_all(&checks).await;
    assert!(finding(&before, "telemetry.subscriber")
        .message
        .contains(AWAITING));

    policy_cell.set(Arc::new(a_policy()));
    report_cell.set(Arc::new(a_report()));

    let after = run_all(&checks).await;
    let subscriber = finding(&after, "telemetry.subscriber");
    assert_eq!(
        subscriber.severity,
        CheckSeverity::Ok,
        "after the write the check must report on the subscriber outcome the \
         report carries, not on the absence of a report: {}",
        subscriber.message
    );
    assert!(
        !subscriber.message.contains(AWAITING),
        "stale answer after the write: {}",
        subscriber.message
    );

    let store = finding(&after, "telemetry.store");
    assert!(
        !store.message.contains(AWAITING),
        "telemetry.store did not see the late write: {}",
        store.message
    );
}

#[tokio::test]
async fn the_seed_a_check_was_registered_with_is_replaced_by_startups_resolution() {
    // `build` seeds the policy cell with the pre-consent resolution. A check
    // must report the post-consent one — otherwise `doctor` describes a level
    // the person changed away from.
    let policy_cell = StartupCell::filled(Arc::new(policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| p.attribution = cli_framework::telemetry::Attribution::Anonymous,
    )));
    let checks = telemetry_checks(policy_cell.clone(), StartupCell::<StartupReport>::empty());

    let before = run_all(&checks).await;
    let seeded = finding(&before, "telemetry.identity").message.clone();

    policy_cell.set(Arc::new(policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| p.attribution = cli_framework::telemetry::Attribution::Identified,
    )));

    let after = finding(&run_all(&checks).await, "telemetry.identity")
        .message
        .clone();
    assert_ne!(
        seeded, after,
        "telemetry.identity answered from the build-time seed after startup \
         had written the real resolution"
    );
}

// ── part 4: the seam, observed from outside ──────────────────────────────

/// An end-user app whose telemetry settings file lives under `dir`, built the
/// way an application author would build it — no doctor wiring at all.
fn harness(dir: &Path) -> CliTestHarness<TestCtx> {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::EndUser { privacy_url: None })
        .with_telemetry_config_dir(dir)
        .build(TestCtx)
        .unwrap();
    CliTestHarness::new(app)
}

fn doctor_json(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout)
        .unwrap_or_else(|e| panic!("doctor --json is not JSON: {e}\n{stdout}"))
}

#[tokio::test]
async fn an_app_that_wires_no_doctor_checks_still_answers_about_its_telemetry() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());

    let out = h.run(&["demo", "doctor", "--json"]).await;
    assert_eq!(out.exit_code, 0, "doctor failed: {}", out.stderr);

    let report = doctor_json(&out.stdout);
    let reported: Vec<String> = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .map(|f| f["check_id"].as_str().unwrap_or_default().to_string())
        .collect();

    for id in [
        "telemetry.subscriber",
        "telemetry.store",
        "telemetry.endpoint",
        "telemetry.policy",
        "telemetry.identity",
        "telemetry.env",
    ] {
        assert!(
            reported.iter().any(|r| r == id),
            "{id} never reached the user; got {reported:?}"
        );
    }
}

#[tokio::test]
async fn the_report_backed_checks_describe_the_run_that_actually_happened() {
    // The end-to-end proof that `App::init_telemetry` fills the same cells
    // `build` handed the checks. Without that write these three would each
    // carry the awaiting-startup message, and the app's `doctor` would be
    // permanently unable to say anything about its own telemetry.
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());

    let out = h.run(&["demo", "doctor", "--json"]).await;
    let report = doctor_json(&out.stdout);
    let findings = report["findings"].as_array().expect("findings array");

    for id in ["telemetry.subscriber", "telemetry.store", "telemetry.env"] {
        let f = findings
            .iter()
            .find(|f| f["check_id"] == id)
            .unwrap_or_else(|| panic!("{id} missing"));
        let message = f["message"].as_str().unwrap_or_default();
        assert!(
            !message.contains(AWAITING),
            "{id} still waiting on a startup that already ran: {message}"
        );
    }
}

#[tokio::test]
async fn the_store_check_names_the_isolated_directory_the_run_actually_used() {
    // Stronger than "not skipped": the finding has to describe *this* run.
    // `with_telemetry_config_dir` points the store at a temp directory, and
    // only a check reading startup's report — not the build-time seed, which
    // has no report at all — can name it.
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());

    let out = h.run(&["demo", "doctor", "--json"]).await;
    let report = doctor_json(&out.stdout);
    let store = report["findings"]
        .as_array()
        .expect("findings array")
        .iter()
        .find(|f| f["check_id"] == "telemetry.store")
        .expect("telemetry.store missing");

    let rendered = format!("{} {}", store["message"], store["detail"]);
    assert!(
        rendered.contains(dir.path().to_str().expect("utf-8 temp path")),
        "telemetry.store did not name the directory this run used: {rendered}"
    );
}
