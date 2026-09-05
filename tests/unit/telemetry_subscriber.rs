//! The parts of subscriber composition that install nothing.
//!
//! Anything that actually calls `try_init` lives in its own test target,
//! because subscriber installation is a process-global one-way door and two
//! such tests in one binary would each see the other's install.

use cli_framework::doctor::CheckSeverity;
use cli_framework::telemetry::{
    foreign_subscriber_finding, warn_once_foreign_subscriber, SubscriberOutcome,
};
use std::sync::{Arc, Mutex};

#[test]
fn a_foreign_subscriber_is_a_warning_and_never_an_error() {
    let finding = foreign_subscriber_finding();
    assert_eq!(finding.check_id, "telemetry.subscriber");
    assert_eq!(
        finding.severity,
        CheckSeverity::Warning,
        "an application that installed its own subscriber is doing something \
         legitimate; telemetry degrades rather than failing the program"
    );
}

#[test]
fn the_foreign_subscriber_finding_says_what_still_works() {
    let finding = foreign_subscriber_finding();
    let text = format!("{} {}", finding.message, finding.detail.unwrap_or_default());
    assert!(
        text.contains("metric") || text.contains("Metric"),
        "the finding must say metrics are unaffected, since they are: {text}"
    );
    assert!(finding.remediation.is_some());
}

#[test]
fn the_foreign_subscriber_warning_is_emitted_exactly_once_per_process() {
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let seen = seen.clone();
        move |line: &str| seen.lock().unwrap().push(line.to_string())
    };
    for _ in 0..5 {
        warn_once_foreign_subscriber(&sink);
    }
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "a warning repeated on every span would be worse than the problem it reports"
    );
}

#[test]
fn the_outcomes_are_distinguishable_by_the_caller() {
    assert_ne!(
        SubscriberOutcome::Installed,
        SubscriberOutcome::ForeignSubscriber
    );
}
