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

/// The exact set of keys a builder emitted, order-independent.
///
/// Used (per Task 20's negative-check requirement) wherever a `.contains`
/// check would pass just as happily on a stale or extra key — sorting turns
/// "these keys are present" into "these keys are present and no others",
/// without coupling the assertion to the builder's internal push order.
fn key_set(attrs: &[opentelemetry::KeyValue]) -> Vec<String> {
    let mut ks = keys(attrs);
    ks.sort();
    ks
}

/// The expected side of a [`key_set`] comparison: sorts so the call site can
/// list keys in whatever order reads best.
fn sorted_strs(xs: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = xs.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn an_http_client_call_records_the_method_and_the_status_and_no_url() {
    let attrs = http_client_attrs("GET", Some(200), None, None);
    let m = map(&attrs);
    assert!(m.contains(&("http.request.method".to_string(), "GET".to_string())));
    assert!(m.contains(&("http.response.status_code".to_string(), "200".to_string())));
    assert_eq!(
        key_set(&attrs),
        sorted_strs(&[
            "cli.probe",
            "http.request.method",
            "http.response.status_code",
        ]),
        "exact key set — no address/port key when neither is supplied"
    );
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
    let attrs = http_client_attrs("GET", Some(200), Some("api.example.com"), Some(443));
    let address = attrs
        .iter()
        .find(|a| a.key.as_str() == "server.address")
        .expect("recorded when supplied");
    assert_eq!(address.value.as_str(), "api.example.com");
    let port = attrs
        .iter()
        .find(|a| a.key.as_str() == "server.port")
        .expect("recorded when supplied");
    assert_eq!(port.value.as_str(), "443");
    assert!(
        !metric_label_is_allowed("server.address"),
        "a hostname on a metric is a fleet inventory"
    );
    assert!(
        !metric_label_is_allowed("server.port"),
        "a port on a metric is still per-endpoint cardinality"
    );
    assert_eq!(
        key_set(&attrs),
        sorted_strs(&[
            "cli.probe",
            "http.request.method",
            "http.response.status_code",
            "server.address",
            "server.port",
        ]),
        "exact key set — the old wrong key http.client.server_address (the PRD's \
         probe id, not a span key) must not reappear"
    );
}

#[test]
fn an_http_client_call_with_no_response_records_no_status() {
    let attrs = http_client_attrs("GET", None, None, None);
    assert!(!keys(&attrs).contains(&"http.response.status_code".to_string()));
    assert_eq!(
        key_set(&attrs),
        sorted_strs(&["cli.probe", "http.request.method"])
    );
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
    let attrs = chat_attrs(3, 1500.0);
    for (key, _) in map(&attrs) {
        assert!(
            !key.contains("prompt") && !key.contains("message") && !key.contains("content"),
            "there is no probe id under which a prompt travels, at any telemetry \
             level including debug: {key}"
        );
    }
    let m = map(&attrs);
    assert!(m.contains(&("cli.chat.turn_count".to_string(), "3".to_string())));
    assert!(m.contains(&("cli.chat.duration_ms".to_string(), "1500".to_string())));
    assert!(
        !keys(&attrs).contains(&"cli.chat.turns".to_string()),
        "cli.chat.turns is the PRD's metric name, not this span's key"
    );
    assert_eq!(
        key_set(&attrs),
        sorted_strs(&["cli.probe", "cli.chat.turn_count", "cli.chat.duration_ms"]),
        "exact key set — the old wrong key cli.chat.turns must not reappear"
    );
}

#[test]
fn an_mcp_session_records_the_tool_name_which_the_server_declares() {
    let attrs = mcp_session_attrs(Some("search"), None);
    assert!(map(&attrs).contains(&("mcp.tool".to_string(), "search".to_string())));
    assert!(
        !keys(&attrs).contains(&"tool".to_string()),
        "tool is the mcp.tool.calls metric's own label key (a separate, explicit \
         KeyValue::new call at the call site) — the span key is mcp.tool, never \
         shared with it"
    );
    assert!(
        metric_label_is_allowed("tool"),
        "a tool name comes from the server's own declared list, so it is bounded \
         (this is the metric's label, unaffected by the span key rename)"
    );
    assert_eq!(
        key_set(&attrs),
        sorted_strs(&["cli.probe", "mcp.tool"]),
        "status is not known when the span is created, so it must be absent here"
    );
}

#[test]
fn an_mcp_session_records_status_once_the_outcome_is_known() {
    // Trap 2: `status` arrives in a second call, after the outcome is known —
    // never fabricated at span-creation time alongside the tool name.
    let attrs = mcp_session_attrs(None, Some("ok"));
    assert!(map(&attrs).contains(&("status".to_string(), "ok".to_string())));
    assert_eq!(key_set(&attrs), sorted_strs(&["cli.probe", "status"]));
}

#[test]
fn a_doctor_run_records_the_check_id_and_the_severity() {
    let attrs = doctor_attrs("telemetry.store", "warning", 4);
    let m = map(&attrs);
    assert!(m.contains(&("check".to_string(), "telemetry.store".to_string())));
    assert!(m.contains(&("severity".to_string(), "warning".to_string())));
    assert!(m.contains(&("cli.doctor.findings_count".to_string(), "4".to_string())));
    assert_eq!(
        key_set(&attrs),
        sorted_strs(&[
            "cli.probe",
            "check",
            "severity",
            "cli.doctor.findings_count",
        ])
    );
}

#[test]
fn a_process_probe_records_the_exit_code() {
    let attrs = process_attrs(2);
    assert!(map(&attrs).contains(&("process.exit.code".to_string(), "2".to_string())));
    assert_eq!(
        key_set(&attrs),
        sorted_strs(&["cli.probe", "process.exit.code"])
    );
}

#[test]
fn a_config_probe_records_schema_version_backend_and_policy_state() {
    let attrs = config_attrs(3, "registry", "managed");
    let m = map(&attrs);
    assert!(m.contains(&("config.schema_version".to_string(), "3".to_string())));
    assert!(m.contains(&("config.backend".to_string(), "registry".to_string())));
    assert!(m.contains(&("config.policy.state".to_string(), "managed".to_string())));
    assert_eq!(
        key_set(&attrs),
        sorted_strs(&[
            "cli.probe",
            "config.schema_version",
            "config.backend",
            "config.policy.state",
        ])
    );
}

#[test]
fn a_secrets_probe_records_backend_op_and_status() {
    let attrs = secrets_attrs("keychain", "set", "ok");
    let m = map(&attrs);
    assert!(m.contains(&("secrets.backend".to_string(), "keychain".to_string())));
    assert!(m.contains(&("secrets.op".to_string(), "set".to_string())));
    assert!(m.contains(&("status".to_string(), "ok".to_string())));
    assert_eq!(
        key_set(&attrs),
        sorted_strs(&["cli.probe", "secrets.backend", "secrets.op", "status"])
    );
}

#[test]
fn every_probe_function_declares_its_probe_id() {
    let cases: Vec<(&str, Vec<opentelemetry::KeyValue>)> = vec![
        ("cli.process", process_attrs(0)),
        ("cli.help", help_attrs(Some("build"))),
        ("cli.auth", auth_attrs("login", "ok")),
        ("cli.config", config_attrs(1, "file", "managed")),
        ("cli.secrets", secrets_attrs("keychain", "get", "ok")),
        ("cli.doctor", doctor_attrs("x", "ok", 0)),
        ("cli.plugin", plugin_attrs("p", "load")),
        ("cli.chat", chat_attrs(1, 10.0)),
        (
            "http.client",
            http_client_attrs("GET", Some(200), None, None),
        ),
        ("http.server", http_server_attrs("/x", "GET", 200)),
        ("mcp.session", mcp_session_attrs(None, None)),
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
        process_attrs(1),
        help_attrs(Some("b")),
        auth_attrs("login", "ok"),
        config_attrs(1, "file", "managed"),
        secrets_attrs("keychain", "get", "ok"),
        doctor_attrs("x", "ok", 1),
        plugin_attrs("p", "load"),
        chat_attrs(1, 10.0),
        http_client_attrs("GET", Some(200), Some("h"), Some(443)),
        http_server_attrs("/x", "GET", 200),
        mcp_session_attrs(Some("t"), Some("ok")),
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

/// The `cli.secrets` probe's own key names survive the never-list.
///
/// `NEVER_LIST` matches `secret` as a *substring*, which fires on
/// `secrets.backend` and `secrets.op` — the probe family's own name, not a
/// credential. Spec 025 row `cli.secrets` asks for exactly these two keys and
/// then says "never names or values", so the keys are the safe part and the
/// collision is the heuristic overreaching. `NEVER_LIST_EXEMPT` carves them
/// out the same way it already carves out `cli.usage_error.token`.
#[test]
fn the_secrets_probes_own_key_names_survive_the_never_lists_substring_rule() {
    let attrs = secrets_attrs("keychain", "get", "ok");
    for key in ["secrets.backend", "secrets.op"] {
        assert!(
            attrs.iter().any(|a| a.key.as_str() == key),
            "{key} must be emitted by the builder (PRD 329)"
        );
        assert!(
            !is_never_listed(key, &[]),
            "{key} names the backend and the operation, never a secret"
        );
    }

    // The exemption must not disarm the heuristic it carves out of: a key
    // that really does look like a credential is still dropped.
    for key in ["client_secret", "secrets.value", "app.secret", "secret"] {
        assert!(
            is_never_listed(key, &[]),
            "{key} must still be caught — the exemption is two exact keys, not \
             a hole in the *secret* rule"
        );
    }

    // And an author who extends the never-list outranks the framework's
    // judgement about its own keys, because extending it is a deliberate
    // decision that this product cannot carry the value.
    let extended = vec!["secret".to_string()];
    for key in ["secrets.backend", "secrets.op"] {
        assert!(
            is_never_listed(key, &extended),
            "{key}: with_telemetry_never must win over NEVER_LIST_EXEMPT"
        );
    }
}

/// The normative `process.*`, `config.*` and `secrets.*` attributes reach the
/// boundary without the consuming application allowlisting them by hand.
///
/// `keeps_attribute` requires a non-never-listed key to be *framework*-owned
/// before a level check runs. Spec 025 writes these keys without a `cli.`
/// prefix (rows `cli.process`, `cli.config`, `cli.secrets`), so until
/// `FRAMEWORK_PREFIXES` learned `process.`, `config.` and `secrets.` the
/// framework's own attributes were treated as application attributes and
/// dropped by default — the builders were catalogued and unreachable.
#[test]
fn the_normative_process_config_and_secrets_keys_are_framework_keys() {
    let rules = RedactionRules {
        level: TelemetryLevel::Diagnostic,
        app_attr_allowlist: vec![],
        extra_never: vec![],
    };
    for key in [
        "process.exit.code",
        "config.schema_version",
        "config.backend",
        "config.policy.state",
        "secrets.backend",
        "secrets.op",
    ] {
        assert!(
            rules.keeps_attribute(key),
            "{key} is a framework key from spec 025's probe table and must not \
             need the app's own with_telemetry_attrs allowlist"
        );
    }

    // Widening a prefix must not reach past the never-list, which runs first.
    // `process.command_line` is on NEVER_KEYS — the specification forbids it
    // at any level — and adding `process.` to the prefix list must not have
    // made it reachable.
    assert!(
        is_never_listed("process.command_line", &[]),
        "the command line stays forbidden at every level; the prefix widening \
         governs rule 3, never rule 1"
    );
    assert!(
        !rules.keeps_attribute("process.command_line"),
        "and the whole decision must agree with the never-list"
    );

    // `off` still records nothing, prefix or not.
    let off = RedactionRules {
        level: TelemetryLevel::Off,
        app_attr_allowlist: vec![],
        extra_never: vec![],
    };
    for key in ["process.exit.code", "config.backend", "secrets.op"] {
        assert!(!off.keeps_attribute(key), "{key} must be dropped at off");
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
