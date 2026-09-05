// tests/unit/telemetry_pipeline.rs
use cli_framework::telemetry::{sampler_for_policy, Deployment, FlushOutcome, TelemetryLevel};
use std::time::Duration;

mod support;
use support::policy_with;

fn sampler_description(policy: &cli_framework::telemetry::TelemetryPolicy) -> String {
    use opentelemetry_sdk::trace::ShouldSample;
    format!("{:?}", sampler_for_policy(policy))
}

#[test]
fn an_end_user_install_samples_every_trace() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |_| {},
    );
    assert!(
        sampler_description(&policy).contains("AlwaysOn"),
        "one person's handful of invocations is not a volume problem, and a \
         sampled-away trace is a support case nobody can answer: {}",
        sampler_description(&policy)
    );
}

#[test]
fn a_service_samples_by_ratio() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Diagnostic, |p| {
        p.sample_ratio = 0.25;
    });
    let described = sampler_description(&policy);
    assert!(described.contains("ParentBased"), "got {described}");
    assert!(described.contains("0.25"), "got {described}");
}

#[test]
fn debug_forces_full_sampling_even_on_a_service_with_a_low_ratio() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Debug, |p| {
        p.sample_ratio = 0.01;
    });
    assert!(
        sampler_description(&policy).contains("AlwaysOn"),
        "a debug session that drops the trace being debugged is worse than useless"
    );
}

#[test]
fn a_flush_that_finishes_inside_the_budget_reports_completion() {
    let outcome =
        cli_framework::telemetry::flush_within_for_test(Duration::from_millis(500), || {});
    assert_eq!(outcome, FlushOutcome::Completed);
}

#[test]
fn a_flush_that_overruns_the_budget_reports_a_timeout_rather_than_blocking() {
    let started = std::time::Instant::now();
    let outcome =
        cli_framework::telemetry::flush_within_for_test(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(5))
        });
    assert_eq!(outcome, FlushOutcome::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the budget must bound the wait: waited {:?}",
        started.elapsed()
    );
}

#[test]
fn the_metric_view_keeps_only_allowlisted_labels() {
    let kept = cli_framework::telemetry::view_keys_for_test(&[
        "command",
        "status",
        "cli.install.id",
        "path",
    ]);
    assert_eq!(kept, vec!["command".to_string(), "status".to_string()]);
}
