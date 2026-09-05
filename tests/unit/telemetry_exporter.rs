// tests/unit/telemetry_exporter.rs
use cli_framework::telemetry::{
    redact_span, span_verdict, Deployment, RedactingExporter, SpanVerdict, TelemetryLevel,
};
use opentelemetry_sdk::trace::SpanExporter;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod support;
use support::{policy_with, span_named};

/// A `SpanExporter` that records how many times each trait method was called,
/// so a test can prove `RedactingExporter` actually delegates to its inner
/// exporter — rather than merely compiling against the trait — and can tell
/// "never called" apart from "called with nothing to do", which a
/// names-only recorder like `support::RecordingExporter` cannot.
#[derive(Debug, Default)]
struct ProbeExporter {
    export_calls: Arc<Mutex<u32>>,
    shutdown_with_timeout_calls: Arc<Mutex<Vec<Duration>>>,
    shutdown_calls: Arc<Mutex<u32>>,
    force_flush_calls: Arc<Mutex<u32>>,
    set_resource_calls: Arc<Mutex<u32>>,
}

impl SpanExporter for ProbeExporter {
    fn export(
        &self,
        _batch: Vec<opentelemetry_sdk::trace::SpanData>,
    ) -> impl std::future::Future<Output = opentelemetry_sdk::error::OTelSdkResult> + Send {
        let calls = self.export_calls.clone();
        async move {
            *calls.lock().unwrap() += 1;
            Ok(())
        }
    }
    fn shutdown_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> opentelemetry_sdk::error::OTelSdkResult {
        self.shutdown_with_timeout_calls
            .lock()
            .unwrap()
            .push(timeout);
        Ok(())
    }
    fn shutdown(&mut self) -> opentelemetry_sdk::error::OTelSdkResult {
        *self.shutdown_calls.lock().unwrap() += 1;
        Ok(())
    }
    fn force_flush(&mut self) -> opentelemetry_sdk::error::OTelSdkResult {
        *self.force_flush_calls.lock().unwrap() += 1;
        Ok(())
    }
    fn set_resource(&mut self, _resource: &opentelemetry_sdk::Resource) {
        *self.set_resource_calls.lock().unwrap() += 1;
    }
}

#[test]
fn a_span_whose_probe_is_below_the_telemetry_level_is_dropped_whole() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Usage, |_| {});
    assert_eq!(span_verdict(&policy, Some("cli.config")), SpanVerdict::Drop);
    assert_eq!(
        span_verdict(&policy, Some("cli.command")),
        SpanVerdict::Keep
    );
}

#[test]
fn a_span_whose_probe_was_switched_off_is_dropped_even_at_debug() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Debug, |p| {
        p.disabled_probes.insert("cli.config".to_string());
    });
    assert_eq!(span_verdict(&policy, Some("cli.config")), SpanVerdict::Drop);
}

#[test]
fn switching_a_parent_probe_off_drops_its_children_too() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Debug, |p| {
        p.disabled_probes.insert("cli.command".to_string());
    });
    assert_eq!(
        span_verdict(&policy, Some("cli.command.args")),
        SpanVerdict::Drop
    );
}

#[test]
fn a_span_that_declares_no_probe_is_dropped_rather_than_guessed_at() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Debug, |_| {});
    assert_eq!(
        span_verdict(&policy, None),
        SpanVerdict::Drop,
        "an unlabelled span is a probe someone forgot to declare; exporting it \
         would mean shipping data no probe catalog describes"
    );
}

#[test]
fn every_span_is_dropped_when_the_telemetry_level_is_off() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Off, |_| {});
    assert_eq!(
        span_verdict(&policy, Some("cli.command")),
        SpanVerdict::Drop
    );
}

#[test]
fn a_kept_span_loses_the_attributes_its_telemetry_level_does_not_permit() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Usage, |_| {});
    let mut span = span_named(
        "cli.command",
        &[
            ("cli.probe", "cli.command"),
            ("command", "build"),
            ("error.type", "io"),
            ("api_key", "sk-1"),
        ],
    );
    assert_eq!(redact_span(&policy, &mut span), SpanVerdict::Keep);
    let keys: Vec<String> = span.attributes.iter().map(|a| a.key.to_string()).collect();
    assert_eq!(keys, vec!["command".to_string()], "got {keys:?}");
}

#[test]
fn the_routing_attribute_is_stripped_from_the_exported_span() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Debug, |_| {});
    let mut span = span_named(
        "cli.command",
        &[("cli.probe", "cli.command"), ("command", "b")],
    );
    redact_span(&policy, &mut span);
    let keys: Vec<String> = span.attributes.iter().map(|a| a.key.to_string()).collect();
    assert!(
        !keys.contains(&"cli.probe".to_string()),
        "cli.probe is routing information for the boundary, not data: {keys:?}"
    );
}

#[test]
fn an_event_belonging_to_a_switched_off_probe_is_removed_from_a_kept_span() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Debug, |p| {
        p.disabled_probes.insert("cli.usage_error".to_string());
    });
    let mut span = span_named("cli.command", &[("cli.probe", "cli.command")]);
    span.events.events.push(support::event(
        "cli.usage_error",
        &[("cli.probe", "cli.usage_error")],
    ));
    span.events
        .events
        .push(support::event("cli.help", &[("cli.probe", "cli.help")]));

    assert_eq!(redact_span(&policy, &mut span), SpanVerdict::Keep);
    let names: Vec<String> = span
        .events
        .events
        .iter()
        .map(|e| e.name.to_string())
        .collect();
    assert_eq!(names, vec!["cli.help".to_string()], "got {names:?}");
}

#[test]
fn an_events_attributes_are_redacted_by_the_same_rules_as_a_spans() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Usage, |_| {});
    let mut span = span_named("cli.command", &[("cli.probe", "cli.command")]);
    span.events.events.push(support::event(
        "cli.panic",
        &[
            ("cli.probe", "cli.panic"),
            ("panic.location", "src/main.rs:12"),
            ("panic.message", "index out of bounds"),
        ],
    ));
    redact_span(&policy, &mut span);
    let keys: Vec<String> = span.events.events[0]
        .attributes
        .iter()
        .map(|a| a.key.to_string())
        .collect();
    assert!(keys.contains(&"panic.location".to_string()));
    assert!(
        !keys.contains(&"panic.message".to_string()),
        "a panic message is debug-only: it quotes program data verbatim"
    );
}

#[test]
fn the_exporter_forwards_only_what_survives() {
    let policy = Arc::new(policy_with(
        Deployment::Service,
        TelemetryLevel::Usage,
        |_| {},
    ));
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let inner = support::RecordingExporter::new(seen.clone());
    let exporter = RedactingExporter::new(inner, policy);

    let batch = vec![
        span_named("kept", &[("cli.probe", "cli.command")]),
        span_named("dropped", &[("cli.probe", "cli.config")]),
        span_named("unlabelled", &[]),
    ];
    support::export_blocking(&exporter, batch);

    assert_eq!(&*seen.lock().unwrap(), &vec!["kept".to_string()]);
}

#[test]
fn an_empty_surviving_batch_never_reaches_the_inner_exporter() {
    let policy = Arc::new(policy_with(
        Deployment::Service,
        TelemetryLevel::Usage,
        |_| {},
    ));
    let probe = ProbeExporter::default();
    let export_calls = probe.export_calls.clone();
    let exporter = RedactingExporter::new(probe, policy);

    // Every span here is below the telemetry level, so the boundary drops
    // the whole batch. The inner exporter must never be called at all — not
    // even once with an empty `Vec` — because a real OTLP exporter turns
    // every call into an HTTP request, and "send nothing" is not the same
    // as "send an empty batch" from a collector's point of view.
    let batch = vec![span_named("dropped", &[("cli.probe", "cli.config")])];
    support::export_blocking(&exporter, batch);

    assert_eq!(
        *export_calls.lock().unwrap(),
        0,
        "every span was dropped by the boundary, so the inner exporter's \
         export() must never run"
    );
}

#[test]
fn the_exporter_delegates_shutdown_flush_and_resource_calls_to_the_inner_exporter() {
    let policy = Arc::new(policy_with(
        Deployment::Service,
        TelemetryLevel::Usage,
        |_| {},
    ));
    let probe = ProbeExporter::default();
    let shutdown_with_timeout_calls = probe.shutdown_with_timeout_calls.clone();
    let shutdown_calls = probe.shutdown_calls.clone();
    let force_flush_calls = probe.force_flush_calls.clone();
    let set_resource_calls = probe.set_resource_calls.clone();
    let mut exporter = RedactingExporter::new(probe, policy);

    assert!(exporter
        .shutdown_with_timeout(Duration::from_secs(3))
        .is_ok());
    assert_eq!(
        &*shutdown_with_timeout_calls.lock().unwrap(),
        &vec![Duration::from_secs(3)],
        "the timeout must reach the inner exporter unchanged"
    );

    assert!(exporter.shutdown().is_ok());
    assert_eq!(*shutdown_calls.lock().unwrap(), 1);

    assert!(exporter.force_flush().is_ok());
    assert_eq!(*force_flush_calls.lock().unwrap(), 1);

    let resource = opentelemetry_sdk::Resource::builder_empty().build();
    exporter.set_resource(&resource);
    assert_eq!(*set_resource_calls.lock().unwrap(), 1);
}
