//! One test, one process: installing the default logging subscriber is a
//! process-global one-way door, same reasoning as
//! `unit_telemetry_subscriber_install`. This covers `install_default_logging`,
//! `LoggingGuard`, and the reload slot a later OTel layer attaches to —
//! nothing in `unit_telemetry_subscriber*` exercises this path, since those
//! only call `install_subscriber_for_test`.
//!
//! Added alongside the plan's given tests rather than ahead of the
//! implementation: the plan's Task 14 code blocks did not specify this
//! reload-slot mechanism as a concrete test, only as prose ("a reload handle
//! ... PR7 wires it up"), and it is not exercised by any of the three tests
//! the plan does give verbatim. Without it, `LoggingGuard::attach_otel_layer`
//! and the `Some(ReloadSlot(..))` arm of `install_default_logging` would have
//! no test coverage at all.

use cli_framework::telemetry::BoxedLayer;

#[test]
fn the_reload_slot_attaches_a_layer_when_the_install_won_and_no_ops_when_it_lost() {
    let first = cli_framework::init_default_logging();
    assert!(
        first.can_attach_otel_layer(),
        "the first install in the process should win the process global and leave a reload slot"
    );
    let layer: BoxedLayer = Box::new(tracing_subscriber::layer::Identity::new());
    assert!(
        first.attach_otel_layer(layer).is_ok(),
        "attaching to a live reload slot must succeed"
    );

    let second = cli_framework::init_default_logging();
    assert!(
        !second.can_attach_otel_layer(),
        "a second install in the same process loses the process global, so there is no slot \
         to attach to"
    );
    let layer: BoxedLayer = Box::new(tracing_subscriber::layer::Identity::new());
    assert!(
        second.attach_otel_layer(layer).is_ok(),
        "attaching with no slot is a no-op, not an error — the same degrade-don't-fail rule \
         as a foreign subscriber"
    );
}
