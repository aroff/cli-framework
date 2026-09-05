// tests/unit/telemetry_exporter.rs
use cli_framework::telemetry::{
    redact_span, span_verdict, Deployment, KeyValue, SpanVerdict, TelemetryLevel,
};

mod support;
use support::{policy_with, span_named};

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
    use cli_framework::telemetry::RedactingExporter;
    use std::sync::{Arc, Mutex};

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
