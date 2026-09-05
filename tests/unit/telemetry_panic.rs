//! The `cli.panic` probe: location always, message only at debug.
//!
//! This whole file mutates the process-global panic hook via
//! `std::panic::set_hook`/`take_hook`. Rust's test harness runs `#[test]`
//! functions on multiple threads by default, and two tests each swapping the
//! *same* global hook in and out at once would interleave and could capture
//! each other's panic. The plan calls for `#[serial_test::serial]` here, but
//! falling back to hand serialization when the crate has no `serial_test`
//! dev-dependency — confirmed absent by grep before writing this file, so no
//! dependency is added. Every test instead takes `HOOK_LOCK` for its
//! duration, which has the same effect with no new dependency.
//!
//! Three of the plan's seven given tests
//! (`the_location_is_reported_at_usage_but_the_message_is_not`,
//! `the_message_is_reported_at_debug`,
//! `a_panic_message_that_quotes_a_secret_is_still_dropped_at_debug`) are
//! deferred to PR5 per the plan's explicit instruction: they assert against
//! `RedactionRules`, which lives in `redact.rs`. PR3 (a sibling branch) has
//! not merged into this branch's base — confirmed by `ls src/telemetry/
//! redact.rs` (not found) and a `mod.rs` grep (no `RedactionRules`) before
//! writing this file — so `redact.rs` does not exist here and this file
//! cannot import `RedactionRules`. Panic attribute levels are asserted in
//! PR5, which is the first branch where both `panic.rs` and `redact.rs`
//! exist.

use cli_framework::telemetry::PanicRecord;
use std::sync::{Arc, Mutex};

/// Serializes every test in this file: two tests each swapping the same
/// global panic hook in and out at once would race, since the hook is
/// process-wide state. Recovers from poisoning (`unwrap_or_else`) because an
/// assertion failure under the lock must not wedge every test that runs
/// after it — the guarded data is a plain `()`, so a poisoned lock still
/// guards correctly.
static HOOK_LOCK: Mutex<()> = Mutex::new(());

fn lock_hook() -> std::sync::MutexGuard<'static, ()> {
    HOOK_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Build a `PanicRecord` the way the hook would, by actually panicking inside
/// a caught hook. Nothing else produces a real `PanicHookInfo`.
fn record_of(payload: &'static str) -> PanicRecord {
    let captured: Arc<Mutex<Option<PanicRecord>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        *sink.lock().unwrap() = Some(cli_framework::telemetry::panic_record(info));
    }));
    let _ = std::panic::catch_unwind(|| panic!("{}", payload));
    std::panic::set_hook(previous);

    // Not `captured.lock().unwrap().take().expect(...)` as a single tail
    // expression: rustc extends that `MutexGuard` temporary's lifetime to the
    // end of this block (tail-position temporaries are kept alive alongside
    // the block's locals), which collides with `captured` itself being
    // dropped there — "`captured` does not live long enough" (E0597). Binding
    // the `.take()` result to a local first ends the guard's borrow at the
    // statement, before the return.
    let record = captured.lock().unwrap().take();
    record.expect("the hook ran")
}

#[test]
fn a_panic_record_carries_the_source_location() {
    let _guard = lock_hook();
    let record = record_of("boom");
    let location = record
        .location
        .expect("a panic in Rust always has a location");
    assert!(
        location.contains("telemetry_panic.rs"),
        "the location must name the framework's own source coordinates: {location}"
    );
    assert!(location.contains(':'), "line number expected: {location}");
}

#[test]
fn a_panic_record_carries_the_message_separately_from_the_location() {
    let _guard = lock_hook();
    let record = record_of("index out of bounds");
    assert_eq!(record.message.as_deref(), Some("index out of bounds"));
}

#[test]
fn the_hook_chains_rather_than_replacing_what_was_there() {
    let _guard = lock_hook();
    use std::sync::atomic::{AtomicUsize, Ordering};

    let previous_ran = Arc::new(AtomicUsize::new(0));
    let ours_ran = Arc::new(AtomicUsize::new(0));

    let original = std::panic::take_hook();
    {
        let previous_ran = previous_ran.clone();
        std::panic::set_hook(Box::new(move |_| {
            previous_ran.fetch_add(1, Ordering::SeqCst);
        }));
    }

    {
        let ours_ran = ours_ran.clone();
        cli_framework::telemetry::install_panic_hook(move |_record| {
            ours_ran.fetch_add(1, Ordering::SeqCst);
        });
    }

    let _ = std::panic::catch_unwind(|| panic!("chained"));
    std::panic::set_hook(original);

    assert_eq!(ours_ran.load(Ordering::SeqCst), 1, "our recorder must run");
    assert_eq!(
        previous_ran.load(Ordering::SeqCst),
        1,
        "an application's own panic hook must still run — replacing it would \
         silently remove a crash reporter"
    );
}

#[test]
fn a_non_string_panic_payload_yields_no_message_rather_than_a_placeholder() {
    let _guard = lock_hook();
    let captured: Arc<Mutex<Option<PanicRecord>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        *sink.lock().unwrap() = Some(cli_framework::telemetry::panic_record(info));
    }));
    let _ = std::panic::catch_unwind(|| std::panic::panic_any(42u32));
    std::panic::set_hook(previous);

    let record = captured.lock().unwrap().take().expect("the hook ran");
    assert!(record.location.is_some());
    assert_eq!(
        record.message, None,
        "a placeholder like 'Box<dyn Any>' is noise that looks like data"
    );
}
