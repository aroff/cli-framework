// tests/unit/telemetry_startup_panic_hook.rs
//
//! One test, one process: `install_telemetry_panic_hook` writes the *process*
//! panic hook and `set_global_default` writes the *process* subscriber. Both
//! are one-way doors, and a second test in this binary would observe the
//! first test's hook still chained in front of its own.
//!
//! The subject is the hook's body, which is the one part of startup that no
//! other test can reach: `run_startup` returns long before a panic happens,
//! so `panic_hook: true` in the result proves only that `set_hook` was
//! called. What that hook *records* -- a `cli.panics` count, the source
//! location always, the message only at `debug`, and nothing at all when the
//! `cli.panic` probe is switched off -- is asserted here by panicking for
//! real inside `catch_unwind` and reading the events back off a capturing
//! layer.

use cli_framework::config::resolution::Layer as ConfigLayer;
use cli_framework::config::ConfigFormat;
use cli_framework::telemetry::{
    run_startup, telemetry_only_manifest, Attribution, Deployment, ProbeRegistry, ServiceIdentity,
    StartupInputs, Surface, TelemetryInputs, TelemetryStoreLocation,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

/// A loopback port nothing listens on, so the exporter is built (which is
/// what puts a `Telemetry` handle in the result and therefore installs the
/// hook) without any test waiting on a real socket.
const DEAD_COLLECTOR: &str = "http://127.0.0.1:9/";

/// One captured `tracing` event, as a field map. Field *names* are the whole
/// point here -- `cli.probe`, `panic.location`, `panic.message` -- so the
/// visitor keeps them rather than rendering a message line.
type Fields = BTreeMap<String, String>;

#[derive(Default, Clone)]
struct Captured(Arc<Mutex<Vec<Fields>>>);

impl Captured {
    fn take(&self) -> Vec<Fields> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn panics(&self) -> Vec<Fields> {
        self.take()
            .into_iter()
            .filter(|f| f.get("cli.probe").map(String::as_str) == Some("cli.panic"))
            .collect()
    }
}

struct FieldVisitor(Fields);

impl tracing::field::Visit for FieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        // `%value` in a `tracing` macro arrives here as a `format_args!`,
        // whose `Debug` renders the displayed text without quoting it.
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl<S: tracing::Subscriber> Layer<S> for Captured {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor(Fields::new());
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(visitor.0);
    }
}

/// A `Service` fixture that exports, so `init_from_policy` hands back a
/// handle and step 9 actually installs the hook.
fn exporting_inputs(dir: &Path, env: &[(&str, &str)]) -> StartupInputs {
    let registry = ProbeRegistry::with_builtins();
    let manifest = telemetry_only_manifest("demo", &registry, None);
    StartupInputs {
        base: TelemetryInputs {
            app: "demo".to_string(),
            deployment: Deployment::Service,
            attribution: Attribution::Pseudonymous,
            session_id: "session-fixture".to_string(),
            endpoint: Some(DEAD_COLLECTOR.to_string()),
            endpoint_source: Some(ConfigLayer::Default),
            registry,
            ..Default::default()
        },
        store: TelemetryStoreLocation {
            dir: Some(dir.to_path_buf()),
            format: ConfigFormat::Json,
        },
        manifest: std::sync::Arc::new(manifest),
        service: ServiceIdentity {
            name: "demo".to_string(),
            version: "0.0.0".to_string(),
        },
        surface: Surface::Cli,
        stderr_is_tty: false,
        env: env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

/// Panic on purpose, swallowing the unwind, and hand back what the hook
/// recorded. The default hook is still chained behind ours and prints its
/// usual line to stderr; that is the point of chaining rather than replacing,
/// and a passing test that printed nothing would mean the chain was broken.
fn panic_and_capture<F: FnOnce() + std::panic::UnwindSafe>(seen: &Captured, f: F) -> Vec<Fields> {
    let _ = seen.take();
    let outcome = std::panic::catch_unwind(f);
    assert!(outcome.is_err(), "the fixture must actually have panicked");
    seen.panics()
}

#[test]
fn the_panic_hook_reports_a_crash_at_the_level_each_field_is_allowed() {
    let seen = Captured::default();
    tracing_subscriber::registry()
        .with(seen.clone())
        .try_init()
        .expect("nothing else in this binary installs a subscriber");

    // ---------------------------------------------------------------
    // 1. The probe is switched off: the hook must record nothing at all.
    //    This runs first, and alone, because every `run_startup` below
    //    chains another hook in front of this one -- a later assertion of
    //    "no event" could not tell a silent hook from a missing one.
    // ---------------------------------------------------------------
    let off_dir = tempfile::tempdir().expect("a temp dir");
    let off = run_startup(exporting_inputs(
        off_dir.path(),
        &[("DEMO_TELEMETRY_CLI_PANIC_ENABLED", "0")],
    ));
    assert!(
        off.panic_hook,
        "the fixture must export, or there is no hook under test"
    );
    assert!(
        off.policy.disabled_probes.contains("cli.panic"),
        "`DEMO_TELEMETRY_CLI_PANIC_ENABLED=0` must reach the policy, or the \
         next assertion proves nothing"
    );
    assert!(
        panic_and_capture(&seen, || panic!("suppressed by the probe switch")).is_empty(),
        "a switched-off `cli.panic` probe must emit no event; the switch is \
         the only way a person can stop crash reports and still keep the rest"
    );

    // ---------------------------------------------------------------
    // 2. The probe is on, but the *message* probe is not. A `Service` with
    //    an endpoint resolves to `diagnostic`, where `cli.panic` (usage) is
    //    effective and `cli.panic.message` (debug) is not -- so the crash is
    //    reported and the panic text is held back. This is the arm the
    //    redaction boundary exists for, and it has to run before the `debug`
    //    fixture below, whose hook stays chained in front of it forever.
    // ---------------------------------------------------------------
    let quiet_dir = tempfile::tempdir().expect("a temp dir");
    let quiet = run_startup(exporting_inputs(quiet_dir.path(), &[]));
    assert!(quiet.panic_hook);
    assert!(
        quiet.policy.effective("cli.panic"),
        "the crash report itself must be on at the default `Service` level"
    );
    assert!(
        !quiet.policy.effective("cli.panic.message"),
        "the message probe must be off here, or this scenario is a duplicate \
         of the `debug` one below"
    );

    let events = panic_and_capture(&seen, || panic!("held back below debug"));
    assert_eq!(
        events.len(),
        1,
        "one enabled hook is chained so far, so one event is expected; \
         got {events:?}"
    );
    let event = &events[0];
    assert!(
        event
            .get("panic.location")
            .is_some_and(|l| l.contains("telemetry_startup_panic_hook.rs")),
        "the location must name the panicking source line; got {event:?}"
    );
    assert!(
        !event.contains_key("panic.message"),
        "below `debug` the panic text must not leave the process -- a panic \
         message is arbitrary program data and routinely quotes a path, a \
         URL or a value; got {event:?}"
    );

    // ---------------------------------------------------------------
    // 3. Fully enabled, at `debug`: location *and* message. The hook from
    //    scenario 2 is still chained, so a string panic now produces two
    //    events -- and exactly one of them may carry the message. Asserting
    //    on the split rather than on a count is what makes the level gate
    //    falsifiable: a hook that ignored `cli.panic.message` would put the
    //    text on both.
    // ---------------------------------------------------------------
    let on_dir = tempfile::tempdir().expect("a temp dir");
    let on = run_startup(exporting_inputs(
        on_dir.path(),
        &[("DEMO_TELEMETRY_LEVEL", "debug")],
    ));
    assert!(on.panic_hook);
    assert!(
        on.policy.effective("cli.panic.message"),
        "the message probe is `debug`-only; without it the next assertion \
         would be asserting the wrong branch"
    );

    let events = panic_and_capture(&seen, || panic!("a message worth redacting"));
    assert_eq!(
        events.len(),
        2,
        "two enabled hooks are chained now -- the `diagnostic` one and the \
         `debug` one; got {events:?}"
    );
    assert!(
        events.iter().all(|e| e
            .get("panic.location")
            .is_some_and(|l| l.contains("telemetry_startup_panic_hook.rs"))),
        "every report names the panicking source line; got {events:?}"
    );
    let with_message: Vec<_> = events
        .iter()
        .filter_map(|e| e.get("panic.message"))
        .collect();
    assert_eq!(
        with_message,
        vec!["a message worth redacting"],
        "exactly the `debug` hook may carry the panic text; the `diagnostic` \
         hook must still be holding it back; got {events:?}"
    );

    // ---------------------------------------------------------------
    // 4. A payload that is not a string. `panic_record` reports no message
    //    rather than a `Box<dyn Any>` placeholder that looks like data, and
    //    the hook has to take its no-message arm even at `debug`.
    // ---------------------------------------------------------------
    let events = panic_and_capture(&seen, || std::panic::panic_any(7u32));
    assert_eq!(events.len(), 2, "got {events:?}");
    assert!(
        events.iter().all(|e| e.get("panic.location").is_some()),
        "a non-string payload still has a location; got {events:?}"
    );
    assert!(
        events.iter().all(|e| !e.contains_key("panic.message")),
        "there is no message to report, and an empty or placeholder one \
         would look like a panic that said nothing; got {events:?}"
    );
}
