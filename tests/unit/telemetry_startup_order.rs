//! Pins the fixed order telemetry starts up in (spec: the order is
//! load-bearing, not incidental). Each test below asserts one reason a step
//! must come before another, not just "some order exists."
use cli_framework::telemetry::{startup_order, StartupStep};

#[test]
fn the_kill_switch_check_comes_before_anything_that_touches_the_disk() {
    let order = startup_order();
    let kill = order
        .iter()
        .position(|s| *s == StartupStep::KillSwitches)
        .unwrap();
    let store = order
        .iter()
        .position(|s| *s == StartupStep::OpenStore)
        .unwrap();
    assert!(
        kill < store,
        "DO_NOT_TRACK must cost nothing: no file read, no id minted"
    );
}

#[test]
fn the_policy_is_frozen_before_anything_takes_a_reference_to_it() {
    let order = startup_order();
    let resolve = order
        .iter()
        .position(|s| *s == StartupStep::FreezePolicy)
        .unwrap();
    let providers = order
        .iter()
        .position(|s| *s == StartupStep::BuildProviders)
        .unwrap();
    assert!(
        resolve < providers,
        "the export boundary holds an Arc<TelemetryPolicy> and must never see it change"
    );
}

#[test]
fn the_subscriber_is_installed_after_the_providers_exist() {
    let order = startup_order();
    let providers = order
        .iter()
        .position(|s| *s == StartupStep::BuildProviders)
        .unwrap();
    let subscriber = order
        .iter()
        .position(|s| *s == StartupStep::InstallSubscriber)
        .unwrap();
    assert!(
        providers < subscriber,
        "the OTel layer needs a tracer, which only exists once the provider does"
    );
}

#[test]
fn the_notice_is_shown_before_the_command_runs() {
    let order = startup_order();
    let notice = order
        .iter()
        .position(|s| *s == StartupStep::ShowNotice)
        .unwrap();
    let dispatch = order
        .iter()
        .position(|s| *s == StartupStep::Dispatch)
        .unwrap();
    assert!(
        notice < dispatch,
        "a person must see the notice before the command's own output, not after"
    );
}

#[test]
fn the_manifest_is_merged_before_resolution_reads_its_leaves() {
    let order = startup_order();
    let merge = order
        .iter()
        .position(|s| *s == StartupStep::MergeManifest)
        .unwrap();
    let resolve = order
        .iter()
        .position(|s| *s == StartupStep::Resolve)
        .unwrap();
    assert!(merge < resolve);
}

#[test]
fn dispatch_is_last_and_appears_once() {
    let order = startup_order();
    assert_eq!(order.last(), Some(&StartupStep::Dispatch));
    assert_eq!(
        order
            .iter()
            .filter(|s| **s == StartupStep::Dispatch)
            .count(),
        1
    );
}

#[test]
fn every_step_appears_exactly_once() {
    let order = startup_order();
    let mut seen: Vec<StartupStep> = order.to_vec();
    seen.sort_by_key(|s| format!("{s:?}"));
    seen.dedup();
    assert_eq!(
        seen.len(),
        order.len(),
        "a repeated step is a bug: {order:?}"
    );
}

// Beyond the plan's given 7: `StartupReport` is this task's other product
// (Interfaces section), but none of the plan-given tests above construct one
// — they only exercise `startup_order`/`StartupStep`. Left untested,
// `StartupReport::default()` (and the hand-written `StoreState: Default` it
// depends on, since the plan requires `Unavailable("not opened")` rather than
// a derive) would be new `src/**.rs` lines with no line coverage at all.
#[test]
fn a_default_startup_report_records_nothing_happened_yet() {
    use cli_framework::telemetry::{StartupReport, StoreState, SubscriberOutcome};

    let report = StartupReport::default();

    assert_eq!(
        report.subscriber,
        SubscriberOutcome::Installed,
        "SubscriberOutcome's own default is Installed, the ordinary case"
    );
    assert_eq!(
        report.store,
        StoreState::Unavailable("not opened".to_string()),
        "a default report must not claim the store is ready — that would hide a bug"
    );
    assert_eq!(report.kill_switch, None);
    assert!(report.unmatched_env.is_empty());
    assert!(report.findings.is_empty());
}

// Three orderings carry a stated reason on their `StartupStep` variant but
// were pinned by nothing: each of the three was verified to survive a swap of
// the two steps involved before these were written, which is the only reason
// to add a test to a suite that already has eight.
#[test]
fn the_notice_is_shown_after_the_subscriber_exists_to_log_it() {
    let order = startup_order();
    let subscriber = order
        .iter()
        .position(|s| *s == StartupStep::InstallSubscriber)
        .unwrap();
    let notice = order
        .iter()
        .position(|s| *s == StartupStep::ShowNotice)
        .unwrap();
    assert!(
        subscriber < notice,
        "the notice is emitted through tracing; shown first it would be swallowed \
         by a subscriber that does not exist yet"
    );
}

#[test]
fn the_store_is_opened_before_resolution_needs_its_stored_consent() {
    let order = startup_order();
    let store = order
        .iter()
        .position(|s| *s == StartupStep::OpenStore)
        .unwrap();
    let resolve = order
        .iter()
        .position(|s| *s == StartupStep::Resolve)
        .unwrap();
    assert!(
        store < resolve,
        "resolution reads the stored telemetry.level; resolving first would clamp \
         against an absent config_file layer and silently ignore a person's choice"
    );
}

#[test]
fn the_panic_hook_is_the_last_thing_installed_before_dispatch() {
    let order = startup_order();
    let hook = order
        .iter()
        .position(|s| *s == StartupStep::InstallPanicHook)
        .unwrap();
    assert_eq!(
        hook,
        order.len() - 2,
        "a panic during startup must be reported by whatever hook was already \
         installed, not by a half-built one that references providers that do \
         not exist yet: {order:?}"
    );
}
