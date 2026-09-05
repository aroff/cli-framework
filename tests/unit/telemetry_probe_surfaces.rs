// tests/unit/telemetry_probe_surfaces.rs
//! The remaining probe catalog: http.client, http.server, mcp.session,
//! cli.chat, cli.doctor, cli.plugin, cli.auth, cli.config, cli.secrets,
//! cli.help and cli.process (Task 20).
//!
//! Every function under test is pure — no provider, no collector, no
//! subscriber — so this file asserts directly against the `Vec<KeyValue>`
//! each one returns, plus the redaction-boundary predicates
//! (`is_never_listed`, `metric_label_is_allowed`) and the catalog
//! (`ProbeRegistry::with_builtins`) that govern what happens to those
//! attributes downstream. Nothing here writes `if ctx.telemetry().effective(..)`
//! as a correctness gate — that would be enforcing redaction at the call site,
//! which spec 025 reserves for the export boundary alone.
use cli_framework::telemetry::*;

fn map(attrs: &[opentelemetry::KeyValue]) -> Vec<(String, String)> {
    attrs
        .iter()
        .map(|a| (a.key.to_string(), a.value.as_str().to_string()))
        .collect()
}

fn keys(attrs: &[opentelemetry::KeyValue]) -> Vec<String> {
    attrs.iter().map(|a| a.key.to_string()).collect()
}

#[test]
fn an_http_client_call_records_the_method_and_the_status_and_no_url() {
    let attrs = http_client_attrs("GET", Some(200), None);
    let m = map(&attrs);
    assert!(m.contains(&("http.request.method".to_string(), "GET".to_string())));
    assert!(m.contains(&("http.response.status_code".to_string(), "200".to_string())));
    for forbidden in [
        "url.full",
        "url.path",
        "url.query",
        "http.url",
        "http.target",
    ] {
        assert!(
            !keys(&attrs).contains(&forbidden.to_string()),
            "a URL is where the never-list is useless: the secret is in the \
             value, not in a key called token ({forbidden})"
        );
    }
}

#[test]
fn the_server_address_is_its_own_probe_and_never_a_metric_label() {
    let attrs = http_client_attrs("GET", Some(200), Some("api.example.com"));
    let address = attrs
        .iter()
        .find(|a| a.key.as_str() == "http.client.server_address")
        .expect("recorded when supplied");
    assert_eq!(address.value.as_str(), "api.example.com");
    assert!(
        !metric_label_is_allowed("http.client.server_address"),
        "a hostname on a metric is a fleet inventory"
    );
}

#[test]
fn an_http_client_call_with_no_response_records_no_status() {
    let attrs = http_client_attrs("GET", None, None);
    assert!(!keys(&attrs).contains(&"http.response.status_code".to_string()));
}

#[test]
fn an_http_server_request_records_the_route_template_not_the_path() {
    let attrs = http_server_attrs("/v1/users/{id}", "GET", 200);
    let m = map(&attrs);
    assert!(m.contains(&("http.route".to_string(), "/v1/users/{id}".to_string())));
    assert!(
        !m.iter().any(|(_, v)| v.contains("/v1/users/42")),
        "a concrete path carries the identifier the template exists to hide"
    );
}

#[test]
fn the_chat_probe_has_no_attribute_that_could_hold_prompt_text() {
    let attrs = chat_attrs(3);
    for (key, _) in map(&attrs) {
        assert!(
            !key.contains("prompt") && !key.contains("message") && !key.contains("content"),
            "there is no probe id under which a prompt travels, at any telemetry \
             level including debug: {key}"
        );
    }
    assert!(keys(&attrs).contains(&"cli.chat.turns".to_string()));
}

#[test]
fn an_mcp_session_records_the_tool_name_which_the_server_declares() {
    let attrs = mcp_session_attrs(Some("search"));
    assert!(map(&attrs).contains(&("tool".to_string(), "search".to_string())));
    assert!(
        metric_label_is_allowed("tool"),
        "a tool name comes from the server's own declared list, so it is bounded"
    );
}

#[test]
fn a_doctor_run_records_the_check_id_and_the_severity() {
    let attrs = doctor_attrs("telemetry.store", "warning");
    let m = map(&attrs);
    assert!(m.contains(&("check".to_string(), "telemetry.store".to_string())));
    assert!(m.contains(&("severity".to_string(), "warning".to_string())));
}

#[test]
fn every_probe_function_declares_its_probe_id() {
    let cases: Vec<(&str, Vec<opentelemetry::KeyValue>)> = vec![
        ("cli.process", process_attrs()),
        ("cli.help", help_attrs(Some("build"))),
        ("cli.auth", auth_attrs("login", "ok")),
        ("cli.config", config_attrs("get")),
        ("cli.secrets", secrets_attrs("get")),
        ("cli.doctor", doctor_attrs("x", "ok")),
        ("cli.plugin", plugin_attrs("p", "load")),
        ("cli.chat", chat_attrs(1)),
        ("http.client", http_client_attrs("GET", Some(200), None)),
        ("http.server", http_server_attrs("/x", "GET", 200)),
        ("mcp.session", mcp_session_attrs(None)),
    ];
    for (expected, attrs) in cases {
        let probe = attrs
            .iter()
            .find(|a| a.key.as_str() == "cli.probe")
            .unwrap_or_else(|| panic!("{expected} declares no probe"));
        assert_eq!(probe.value.as_str(), expected);
    }
}

#[test]
fn every_probe_id_these_functions_declare_exists_in_the_catalog() {
    let registry = ProbeRegistry::with_builtins();
    for id in [
        "cli.process",
        "cli.help",
        "cli.auth",
        "cli.config",
        "cli.secrets",
        "cli.doctor",
        "cli.plugin",
        "cli.chat",
        "http.client",
        "http.server",
        "mcp.session",
        "cli.command",
        "cli.usage_error",
        "cli.panic",
        "cli.feature",
    ] {
        assert!(
            registry.get(id).is_some(),
            "{id} is emitted but not catalogued, so it appears in no published \
             list and no telemetry status output"
        );
    }
}

#[test]
fn no_probe_function_emits_a_key_the_never_list_rejects() {
    let all: Vec<opentelemetry::KeyValue> = [
        process_attrs(),
        help_attrs(Some("b")),
        auth_attrs("login", "ok"),
        config_attrs("get"),
        secrets_attrs("get"),
        doctor_attrs("x", "ok"),
        plugin_attrs("p", "load"),
        chat_attrs(1),
        http_client_attrs("GET", Some(200), Some("h")),
        http_server_attrs("/x", "GET", 200),
        mcp_session_attrs(Some("t")),
    ]
    .concat();
    for kv in &all {
        assert!(
            !is_never_listed(kv.key.as_str(), &[]),
            "{} is never-listed and must not be emitted at all",
            kv.key
        );
    }
}

#[test]
fn every_instrument_label_is_in_the_metric_label_allowlist() {
    // PR3 Task 13 builds a View per instrument that keeps only allowlisted
    // keys. An instrument labelled with a key outside that list does not
    // fail: it silently loses the label, and the metric aggregates across a
    // dimension somebody is about to query by.
    use cli_framework::telemetry::{metrics, METRIC_LABEL_ALLOWLIST};
    let declared: &[(&str, &[&str])] = &[
        (
            metrics::COMMAND_INVOCATIONS,
            &["command", "surface", "status"],
        ),
        (
            metrics::COMMAND_DURATION_MS,
            &["command", "surface", "status"],
        ),
        (metrics::PROCESS_DURATION_MS, &["status"]),
        (metrics::USAGE_ERRORS, &["kind"]),
        (metrics::PANICS, &["command"]),
        (metrics::HELP_SHOWN, &["command"]),
        (metrics::FEATURE_USES, &["feature"]),
        (metrics::AUTH_EVENTS, &["kind"]),
        (metrics::DOCTOR_FINDINGS, &["check", "severity"]),
        (metrics::PLUGIN_LOADS, &["plugin", "status"]),
        (metrics::CHAT_TURNS, &[]),
        (metrics::CHAT_SESSIONS, &["status"]),
        (
            metrics::HTTP_CLIENT_REQUEST_DURATION,
            &["http.request.method", "http.response.status_code"],
        ),
        (
            metrics::HTTP_SERVER_REQUEST_DURATION,
            &[
                "http.route",
                "http.request.method",
                "http.response.status_code",
            ],
        ),
        (metrics::MCP_TOOL_CALLS, &["tool", "status"]),
    ];
    assert_eq!(
        declared.len(),
        metrics::ALL.len(),
        "an instrument was added to ALL without declaring its labels here"
    );
    for (name, labels) in declared {
        for label in *labels {
            assert!(
                METRIC_LABEL_ALLOWLIST.contains(label),
                "{name} is labelled {label}, which the View will drop"
            );
        }
    }
}

#[test]
fn only_diagnostic_probes_own_child_spans() {
    // The spec's rule: a usage probe adds to the root span; a child span is
    // diagnostic or above. This is the mechanical form of that sentence.
    use cli_framework::telemetry::{spans, ProbeRegistry, TelemetryLevel};
    let registry = ProbeRegistry::with_builtins();
    for span in spans::CHILDREN {
        let owner = match *span {
            s if s.starts_with("cli.config") => "cli.config",
            s if s.starts_with("cli.secrets") => "cli.secrets",
            s if s.starts_with("cli.plugin") => "cli.plugin",
            s if s.starts_with("http.client") => "http.client",
            _ => "http.server",
        };
        let probe = registry.get(owner).unwrap();
        if owner == "http.server" {
            continue; // the pre-existing root span for the API surface, not a child
        }
        assert!(
            probe.min_level >= TelemetryLevel::Diagnostic,
            "{span} is a child span owned by {owner}, which is {:?}; at usage              a trace must be exactly one span",
            probe.min_level
        );
    }
}
