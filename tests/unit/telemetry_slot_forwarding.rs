// tests/unit/telemetry_slot_forwarding.rs
//
//! The attach slot has to forward *every* `Layer` callback, not just the ones
//! a smoke test happens to reach.
//!
//! `AttachOnceSlot` sits between the bare `Registry` and the OTel bridge that
//! telemetry startup attaches into it. Every call the bridge needs arrives
//! through the slot's own implementation, and each one is an independent
//! forwarding body that can be dropped, mistyped or written as a no-op without
//! breaking a single compile. `unit_telemetry_startup_logging_upgrade` proves
//! the bridge is *attached* -- it opens one span and reads its OTel context --
//! which reaches `on_new_span` and nothing else. A slot that forwarded
//! `on_new_span` and silently swallowed the other seven would pass that test
//! and lose field values, span links and context activation in production.
//!
//! So this drives the callbacks that carry data and asserts on what came out
//! the other end:
//!
//! * `on_record` -- a field recorded after the span opened must appear as an
//!   attribute on the exported span.
//! * `on_follows_from` -- a `follows_from` edge must appear as a link.
//! * `on_enter` / `on_exit` -- the bridge activates the OpenTelemetry context
//!   on enter and pops it on exit, so `Context::current()` is a direct read of
//!   whether those two were forwarded.
//!
//! One test, one process: this installs the process-global subscriber.

use opentelemetry::trace::TraceContextExt;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use std::sync::{Arc, Mutex};
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[derive(Clone, Default, Debug)]
struct TestExporter(Arc<Mutex<Vec<SpanData>>>);

impl SpanExporter for TestExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        self.0.lock().unwrap().extend(batch);
        Ok(())
    }
}

#[test]
fn every_layer_callback_the_bridge_needs_is_forwarded_through_the_slot() {
    // The composed subscriber filters on `RUST_LOG`; pin it so an operator's
    // environment cannot turn these assertions off.
    // SAFETY: single-threaded test setup, before any span is opened.
    unsafe {
        std::env::set_var("RUST_LOG", "info");
    }

    let logging = cli_framework::init_default_logging();
    assert!(
        logging.can_attach_otel_layer(),
        "this test owns the process global, so the guard must hold a live slot"
    );

    let exporter = TestExporter::default();
    let (_handle, guard) =
        cli_framework::telemetry::init::init_with_exporter(exporter.clone(), "slot-forwarding");

    // The public attach API, which is the same slot `run_startup` fills when
    // it finds `init_default_logging` already holding the process global.
    // Taking it directly keeps this test about forwarding: whether startup
    // reaches the slot is `unit_telemetry_startup_logging_upgrade`'s job.
    let layer = cli_framework::telemetry::init::otel_layer::<tracing_subscriber::Registry>(&guard);
    logging
        .attach_otel_layer(Box::new(layer))
        .expect("nothing else in this process has attached a layer");

    // A span that already exists is the precondition for `follows_from`: the
    // edge names a span, so one has to have been opened first.
    let earlier = tracing::info_span!("earlier.work");
    let earlier_span_id = {
        let cx = earlier.context();
        let span_ref = TraceContextExt::span(&cx);
        span_ref.span_context().span_id()
    };

    let root = tracing::info_span!(
        "cli.command",
        // Declared empty and filled in afterwards -- the only way to reach
        // `on_record`. A field given a value in the macro goes through
        // `on_new_span` instead and proves nothing about this path.
        cli.probe = tracing::field::Empty,
    );
    root.record("cli.probe", "cli.command");
    root.follows_from(earlier.id());

    let root_span_id = {
        let cx = root.context();
        let span_ref = TraceContextExt::span(&cx);
        span_ref.span_context().span_id()
    };

    // Before entering, no OpenTelemetry context is active.
    assert!(
        !opentelemetry::Context::current()
            .span()
            .span_context()
            .is_valid(),
        "something had already attached an OpenTelemetry context before the \
         span was entered, so the enter/exit assertions below would not be \
         measuring this span"
    );

    {
        let _entered = root.clone().entered();
        assert_eq!(
            opentelemetry::Context::current()
                .span()
                .span_context()
                .span_id(),
            root_span_id,
            "entering the span did not activate its OpenTelemetry context, so \
             `on_enter` never reached the bridge. Anything the app calls while \
             inside this span -- an outbound HTTP request reading the current \
             context to inject `traceparent`, a manually started child span -- \
             is orphaned from the trace"
        );
    }

    assert!(
        !opentelemetry::Context::current()
            .span()
            .span_context()
            .is_valid(),
        "the OpenTelemetry context stayed attached after the span was exited, \
         so `on_exit` never reached the bridge. The activation stack only \
         grows: every later span in this process would be parented under a \
         span that has already ended"
    );

    drop(root);
    drop(earlier);
    guard.flush();

    let exported = exporter.0.lock().unwrap().clone();
    let names: Vec<String> = exported.iter().map(|s| s.name.to_string()).collect();
    let command = exported
        .iter()
        .find(|s| s.name == "cli.command")
        .unwrap_or_else(|| {
            panic!(
                "the `cli.command` span never reached the exporter; exported spans were {names:?}"
            )
        });

    let probe = command
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "cli.probe")
        .unwrap_or_else(|| {
            let keys: Vec<&str> = command
                .attributes
                .iter()
                .map(|kv| kv.key.as_str())
                .collect();
            panic!(
                "`cli.probe` was recorded on the span after it opened but never \
                 reached the exported span, so `on_record` is not forwarded. \
                 Every attribute set by `Span::record` -- which is how the \
                 framework stamps a probe id, a command status and an exit code \
                 onto the root span -- is being dropped. Attributes present: {keys:?}"
            )
        });
    assert_eq!(
        probe.value.as_str(),
        "cli.command",
        "the recorded value did not survive the round trip"
    );

    let linked: Vec<_> = command
        .links
        .iter()
        .map(|l| l.span_context.span_id())
        .collect();
    assert!(
        linked.contains(&earlier_span_id),
        "the `follows_from` edge never became a link on the exported span, so \
         `on_follows_from` is not forwarded. Links present: {linked:?}"
    );

    drop(logging);
}
