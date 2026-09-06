// tests/unit/telemetry_probe_feature.rs
//
// Task 19 ("Feature marking"). Adapted from the plan's Step 1 listing: the
// plan assumes `FeatureOutcome::Registered { metric: bool }` and
// `feature_outcome(&ProbeRegistry, name)`, but the real, already-shipped
// (PR1) shapes are `FeatureOutcome::Recorded` / `Unregistered` (no fields)
// and `feature_outcome(registered: &[&str], name: &str)` — a plain name
// list, not a registry. `registered_feature_names` is the new bridging
// helper that adapts a `ProbeRegistry` into that shape without redefining
// either PR1 item. Every rewritten test below preserves the plan's original
// intent; only the calling convention changed.
use cli_framework::app::AppContext;
use cli_framework::telemetry::{
    feature_attrs, feature_outcome, registered_feature_names, FeatureOutcome, ProbeRegistry,
};

fn registry_with(names: &[&str]) -> ProbeRegistry {
    let mut registry = ProbeRegistry::with_builtins();
    for name in names {
        registry
            .register_feature(name)
            .expect("a plain feature name is a valid probe id");
    }
    registry
}

fn keys(attrs: &[opentelemetry::KeyValue]) -> Vec<String> {
    attrs.iter().map(|a| a.key.to_string()).collect()
}

#[test]
fn a_registered_feature_becomes_both_an_event_and_a_metric_label() {
    let registry = registry_with(&["export_pdf"]);
    let outcome = feature_outcome(&registered_feature_names(&registry), "export_pdf");
    assert_eq!(outcome, FeatureOutcome::Recorded);
    let attrs = feature_attrs("export_pdf", outcome);
    assert!(keys(&attrs).contains(&"feature".to_string()));
}

#[test]
fn an_unregistered_feature_becomes_an_event_but_never_a_metric_label() {
    let registry = registry_with(&[]);
    let outcome = feature_outcome(
        &registered_feature_names(&registry),
        "whatever_the_user_typed",
    );
    assert_eq!(outcome, FeatureOutcome::Unregistered);
    let attrs = feature_attrs("whatever_the_user_typed", outcome);
    assert!(
        !keys(&attrs).contains(&"feature".to_string()),
        "mark_feature(&user_input) in a loop would mint one time series per \
         input; an event is bounded by its trace and stays useful"
    );
    assert!(
        keys(&attrs).contains(&"cli.feature.name".to_string()),
        "the name still travels as a span attribute: {:?}",
        keys(&attrs)
    );
}

#[test]
fn the_outcome_is_the_same_in_release_as_in_debug() {
    // `feature_outcome` is pure and carries no `cfg(debug_assertions)`; the
    // assertion lives at the callsite (`AppContext::mark_feature`). This test
    // is what makes the release path testable at all — PR1 established the
    // function, this pins that the wiring did not add a debug-only branch.
    let registry = registry_with(&["known"]);
    let names = registered_feature_names(&registry);
    assert_eq!(feature_outcome(&names, "known"), FeatureOutcome::Recorded);
    assert_eq!(
        feature_outcome(&names, "unknown"),
        FeatureOutcome::Unregistered
    );
}

#[test]
fn a_feature_event_always_declares_the_feature_probe() {
    for outcome in [FeatureOutcome::Recorded, FeatureOutcome::Unregistered] {
        let attrs = feature_attrs("x", outcome);
        let probe = attrs
            .iter()
            .find(|a| a.key.as_str() == "cli.probe")
            .unwrap();
        assert_eq!(probe.value.as_str(), "cli.feature");
    }
}

#[test]
fn a_registered_feature_gets_its_own_child_probe_id_so_it_can_be_switched_off() {
    let registry = registry_with(&["export_pdf"]);
    assert!(
        registry.get("cli.feature.export_pdf").is_some(),
        "an author must be able to disable one feature probe without disabling \
         feature marking altogether"
    );
}

#[test]
fn a_feature_name_that_is_not_a_valid_probe_segment_is_rejected_at_registration() {
    let mut registry = ProbeRegistry::with_builtins();
    assert!(
        registry.register_feature("Export PDF").is_err(),
        "a probe id is lower-case dotted; the error belongs at registration, \
         not at every mark_feature call"
    );
}

/// `register_feature`'s own pre-check (empty, or already dotted) is a
/// distinct code path from the one the test above exercises: `"Export PDF"`
/// fails later, inside `validate_probe_id`, on its uppercase letters and
/// space — never touching the `name.is_empty() || name.contains('.')` guard
/// at all. A single-segment name must never be allowed to smuggle in a
/// second segment (`register_feature("a.b")` would otherwise silently mint
/// `cli.feature.a.b`, a probe id nobody asked for), so this guard rejects
/// both malformed shapes before `validate_probe_id` ever runs.
#[test]
fn register_feature_rejects_an_empty_or_dotted_name_before_validating_the_probe_id() {
    use cli_framework::telemetry::ProbeIdError;

    let mut registry = ProbeRegistry::with_builtins();
    assert!(
        matches!(
            registry.register_feature(""),
            Err(ProbeIdError::Malformed(_))
        ),
        "an empty feature name must be rejected by register_feature's own guard"
    );
    assert!(
        matches!(
            registry.register_feature("export.pdf"),
            Err(ProbeIdError::Malformed(_))
        ),
        "a dotted feature name must be rejected before it can mint an extra probe-id segment"
    );
}

// --- Supplementary: the `AppContext::mark_feature` wiring itself. ---
//
// None of the plan's six tests above call `mark_feature` — they exercise the
// pure functions it is built from. Nothing else in the plan or the existing
// suite exercises the wiring (the `opt_probe_registry` accessor, the
// registry lookup, and the emitted event), so these two are added for
// coverage of `src/app/context.rs`'s new code, per this task's coverage
// requirement.
use std::sync::Mutex;

/// Delegates `counter`/`histogram`/`span` to `NoopTelemetry` (the only public
/// way to construct their opaque return types from outside the crate) and
/// captures `event` calls for assertions.
#[derive(Default)]
struct RecordingTelemetry {
    events: Mutex<Vec<(String, Vec<opentelemetry::KeyValue>)>>,
}

impl cli_framework::telemetry::Telemetry for RecordingTelemetry {
    fn event(&self, name: &str, attrs: &[opentelemetry::KeyValue]) {
        self.events
            .lock()
            .unwrap()
            .push((name.to_string(), attrs.to_vec()));
    }
    fn counter(&self, name: &str) -> cli_framework::telemetry::Counter {
        cli_framework::telemetry::NoopTelemetry.counter(name)
    }
    fn histogram(&self, name: &str) -> cli_framework::telemetry::Histogram {
        cli_framework::telemetry::NoopTelemetry.histogram(name)
    }
    fn span(
        &self,
        name: &str,
        attrs: &[opentelemetry::KeyValue],
    ) -> cli_framework::telemetry::SpanHandle {
        cli_framework::telemetry::NoopTelemetry.span(name, attrs)
    }
}

struct FeatureTestCtx {
    registry: ProbeRegistry,
    telemetry: RecordingTelemetry,
}

impl AppContext for FeatureTestCtx {
    fn opt_probe_registry(&self) -> Option<&ProbeRegistry> {
        Some(&self.registry)
    }

    fn telemetry(&self) -> &dyn cli_framework::telemetry::Telemetry {
        &self.telemetry
    }
}

#[test]
fn mark_feature_on_a_registered_name_emits_the_event_and_the_metric_label() {
    let ctx = FeatureTestCtx {
        registry: registry_with(&["export_pdf"]),
        telemetry: RecordingTelemetry::default(),
    };
    ctx.mark_feature("export_pdf");
    let events = ctx.telemetry.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    let (name, attrs) = &events[0];
    assert_eq!(name, "cli.feature");
    assert!(keys(attrs).contains(&"feature".to_string()));
}

#[test]
#[should_panic(expected = "unregistered telemetry feature name")]
fn mark_feature_on_an_unregistered_name_trips_the_debug_assert() {
    let ctx = FeatureTestCtx {
        registry: registry_with(&[]),
        telemetry: RecordingTelemetry::default(),
    };
    // Debug builds run this crate's own test suite, so this is expected to
    // panic here — the release-path outcome (event recorded, one warning, no
    // metric) is asserted above through the pure `feature_outcome`/
    // `feature_attrs` functions instead, per this task's own guidance: the
    // assertion must never depend on build profile.
    ctx.mark_feature("whatever_the_user_typed");
}
