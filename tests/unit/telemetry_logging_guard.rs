//! One test, one process: installing the default logging subscriber is a
//! process-global one-way door, same reasoning as
//! `unit_telemetry_subscriber_install`. This covers `install_default_logging`,
//! `LoggingGuard`, and the reload slot a later OTel layer attaches to —
//! nothing in `unit_telemetry_subscriber*` exercises this path, since those
//! only call `install_subscriber_for_test`.
//!
//! The assertions are on what the attached layer *receives*, not on what
//! `attach_otel_layer` *returns*. An earlier version of this test checked only
//! `is_ok()`, which the whole body of `attach_otel_layer` could be replaced by
//! `Ok(())` without failing — leaving the reload slot, the reason
//! `init_default_logging` returns a guard at all, with no behavioural test.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cli_framework::telemetry::BoxedLayer;
use tracing_subscriber::layer::{Context, Layer};

/// A layer that counts the events the subscriber routes to it. Standing in
/// for the OTel bridge layer, which is what PR7 attaches here for real.
struct CountingLayer(Arc<AtomicUsize>);

impl<S: tracing::Subscriber> Layer<S> for CountingLayer {
    fn on_event(&self, _event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn counter() -> (Arc<AtomicUsize>, BoxedLayer) {
    let seen = Arc::new(AtomicUsize::new(0));
    (seen.clone(), Box::new(CountingLayer(seen)))
}

#[test]
fn the_reload_slot_routes_events_to_an_attached_layer_and_no_ops_when_the_install_lost() {
    let first = cli_framework::init_default_logging();
    assert!(
        first.can_attach_otel_layer(),
        "the first install in the process should win the process global and leave a reload slot"
    );

    let (attached, layer) = counter();
    tracing::info!("emitted before the layer is attached");
    assert_eq!(
        attached.load(Ordering::SeqCst),
        0,
        "the slot starts empty, so an event before the attach must reach nothing"
    );

    first
        .attach_otel_layer(layer)
        .expect("attaching to a live reload slot must succeed");

    tracing::info!("emitted after the layer is attached");
    assert_eq!(
        attached.load(Ordering::SeqCst),
        1,
        "attaching must actually swap the layer into the live subscriber — a slot that \
         accepts the layer and then routes nothing to it exports no traces"
    );

    // A second install loses the process global, so its guard has no slot.
    let second = cli_framework::init_default_logging();
    assert!(
        !second.can_attach_otel_layer(),
        "a second install in the same process loses the process global, so there is no slot \
         to attach to"
    );

    let (ignored, layer) = counter();
    second.attach_otel_layer(layer).expect(
        "attaching with no slot is a no-op, not an error — the same degrade-don't-fail \
                 rule as a foreign subscriber",
    );

    tracing::info!("emitted after the second, slotless attach");
    assert_eq!(
        ignored.load(Ordering::SeqCst),
        0,
        "the slotless guard must genuinely drop the layer, not quietly attach it to the \
         subscriber the first guard owns"
    );
    assert_eq!(
        attached.load(Ordering::SeqCst),
        2,
        "and the layer the winning guard attached must still be receiving events"
    );
}
