//! One test, one process: installing a subscriber is a one-way door.

use cli_framework::telemetry::SubscriberOutcome;

#[test]
fn the_framework_installs_the_subscriber_when_nobody_else_has() {
    let outcome = cli_framework::telemetry::install_subscriber_for_test();
    assert_eq!(outcome, SubscriberOutcome::Installed);

    let second = cli_framework::telemetry::install_subscriber_for_test();
    assert_eq!(
        second,
        SubscriberOutcome::ForeignSubscriber,
        "the second attempt loses to the first — including our own"
    );
}
