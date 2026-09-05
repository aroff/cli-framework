//! One test, one process: a subscriber that was already there wins.

use cli_framework::telemetry::SubscriberOutcome;

#[test]
fn an_application_that_installed_its_own_subscriber_keeps_it() {
    tracing_subscriber::fmt()
        .with_writer(std::io::sink)
        .try_init()
        .expect("this test owns the process global");

    let outcome = cli_framework::telemetry::install_subscriber_for_test();
    assert_eq!(outcome, SubscriberOutcome::ForeignSubscriber);

    tracing::info!("still works");
}
