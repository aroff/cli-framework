//! One test, one process: `install_telemetry_subscriber` claims the process
//! global, so it cannot share a binary with any other test that installs one.
//!
//! This is the only test that exercises the *real* startup path.
//! `unit_telemetry_subscriber_install` calls `install_subscriber_for_test`,
//! which composes the same layers over a no-op `Identity` layer — it proves the
//! win/lose contract but says nothing about whether a span emitted through
//! `tracing` afterwards actually reaches the tracer the guard owns. That is the
//! entire job of `install_telemetry_subscriber`, and the outcome enum it
//! returns cannot tell you it was done: a body that composed the OTel layer and
//! then dropped it on the floor still returns `Installed`.
//!
//! So the assertion is on the exported span, not on the return value.

use cli_framework::telemetry::SubscriberOutcome;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default, Debug)]
struct TestExporter(Arc<Mutex<Vec<SpanData>>>);

impl SpanExporter for TestExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        self.0.lock().unwrap().extend(batch);
        Ok(())
    }
}

#[test]
fn the_installed_subscriber_routes_tracing_spans_to_the_guards_tracer() {
    // The composed subscriber filters on `RUST_LOG`; pin it so an operator's
    // environment cannot turn this assertion off.
    std::env::set_var("RUST_LOG", "info");

    let exporter = TestExporter::default();
    let (_handle, guard) =
        cli_framework::telemetry::init::init_with_exporter(exporter.clone(), "test-service");

    let outcome = cli_framework::telemetry::install_telemetry_subscriber(&guard);
    assert_eq!(
        outcome,
        SubscriberOutcome::Installed,
        "nothing else in this process installs a subscriber, so this must win"
    );

    // Emitted through the *global* dispatcher — no `with_default`, no explicit
    // tracer. If the install did not really wire the OTel layer in, this span
    // goes nowhere and the exporter stays empty.
    {
        let _span = tracing::info_span!("probe.span").entered();
    }
    guard.flush();

    let names: Vec<String> = exporter
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.name.to_string())
        .collect();
    assert!(
        names.iter().any(|n| n == "probe.span"),
        "the span emitted through the global subscriber never reached the guard's tracer, \
         so the composed OTel layer exports nothing — exported spans were {names:?}"
    );

    let second = cli_framework::telemetry::install_telemetry_subscriber(&guard);
    assert_eq!(
        second,
        SubscriberOutcome::ForeignSubscriber,
        "the second attempt loses to the first — including our own"
    );
}
