// tests/unit/telemetry_redact.rs
use cli_framework::telemetry::{
    attribute_min_level, is_never_listed, metric_label_is_allowed, probe_of, Deployment, KeyValue,
    RedactionRules, TelemetryLevel,
};

mod support;
use support::policy_with;

fn rules(level: TelemetryLevel) -> RedactionRules {
    RedactionRules::from_policy(&policy_with(
        Deployment::EndUser { privacy_url: None },
        level,
        |_| {},
    ))
}

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue::new(key.to_string(), value.to_string())
}

fn kept(rules: &RedactionRules, pairs: &[(&str, &str)]) -> Vec<String> {
    let mut attrs: Vec<KeyValue> = pairs.iter().map(|(k, v)| kv(k, v)).collect();
    rules.retain_attributes(&mut attrs);
    attrs.into_iter().map(|a| a.key.to_string()).collect()
}

#[test]
fn the_never_list_matches_anywhere_in_the_key_and_ignores_case() {
    for key in [
        "password",
        "user_password",
        "DB_PASSWORD",
        "client_secret",
        "Secret",
        "access_token",
        "refresh_token",
        "authorization",
        "http.request.header.Authorization",
        "cookie",
        "Set-Cookie",
        "api_key",
        "OPENAI_API_KEY",
    ] {
        assert!(is_never_listed(key, &[]), "{key} must never be recorded");
    }
}

#[test]
fn an_ordinary_key_is_not_caught_by_the_never_list() {
    for key in [
        "command",
        "duration_ms",
        "status",
        "http.route",
        "cli.probe",
    ] {
        assert!(!is_never_listed(key, &[]), "{key} was wrongly rejected");
    }
}

#[test]
fn an_author_may_extend_the_never_list_but_never_shrink_it() {
    let extra = vec!["patient".to_string()];
    assert!(is_never_listed("patient_id", &extra));
    assert!(
        is_never_listed("password", &extra),
        "extending must not replace the built-in list"
    );
}

#[test]
fn the_never_list_wins_at_debug_which_is_the_most_permissive_level() {
    let kept = kept(
        &rules(TelemetryLevel::Debug),
        &[("api_key", "sk-1"), ("command", "build")],
    );
    assert_eq!(kept, vec!["command".to_string()], "debug is not a bypass");
}

#[test]
fn the_never_list_wins_over_an_authors_own_allowlist() {
    let mut r = rules(TelemetryLevel::Debug);
    r.app_attr_allowlist = vec!["session_token".to_string()];
    assert!(
        !r.keeps_attribute("session_token"),
        "an author cannot allowlist their way past the never-list"
    );
}

#[test]
fn an_exception_message_appears_only_at_debug() {
    assert_eq!(
        attribute_min_level("exception.message"),
        TelemetryLevel::Debug
    );
    assert!(!rules(TelemetryLevel::Diagnostic).keeps_attribute("exception.message"));
    assert!(rules(TelemetryLevel::Debug).keeps_attribute("exception.message"));
}

#[test]
fn an_error_type_appears_from_diagnostic_upward() {
    assert_eq!(
        attribute_min_level("error.type"),
        TelemetryLevel::Diagnostic
    );
    assert!(!rules(TelemetryLevel::Usage).keeps_attribute("error.type"));
    assert!(rules(TelemetryLevel::Diagnostic).keeps_attribute("error.type"));
    assert!(rules(TelemetryLevel::Debug).keeps_attribute("error.type"));
}

#[test]
fn a_usage_attribute_survives_every_level_above_off() {
    for level in [
        TelemetryLevel::Usage,
        TelemetryLevel::Diagnostic,
        TelemetryLevel::Debug,
    ] {
        assert!(rules(level).keeps_attribute("command"), "at {level:?}");
    }
}

#[test]
fn nothing_at_all_survives_when_the_telemetry_level_is_off() {
    let kept = kept(
        &rules(TelemetryLevel::Off),
        &[("command", "build"), ("status", "ok")],
    );
    assert!(kept.is_empty(), "got {kept:?}");
}

#[test]
fn an_application_attribute_needs_the_authors_allowlist() {
    let mut r = rules(TelemetryLevel::Usage);
    assert!(
        !r.keeps_attribute("tenant_tier"),
        "an app attribute the author never declared is unreviewed data"
    );
    r.app_attr_allowlist = vec!["tenant_tier".to_string()];
    assert!(r.keeps_attribute("tenant_tier"));
}

#[test]
fn framework_attributes_do_not_need_the_apps_allowlist() {
    let r = rules(TelemetryLevel::Diagnostic);
    for key in [
        "cli.command.name",
        "http.route",
        "mcp.tool.name",
        "otel.status_code",
        "session.id",
        "service.name",
        "error.type",
    ] {
        assert!(r.keeps_attribute(key), "{key} is the framework's own");
    }
}

#[test]
fn the_probe_attribute_names_the_probe_a_span_belongs_to() {
    let attrs = vec![kv("cli.probe", "cli.command"), kv("command", "build")];
    assert_eq!(probe_of(&attrs), Some("cli.command"));
    assert_eq!(probe_of(&[kv("command", "build")]), None);
}

#[test]
fn the_metric_label_allowlist_is_closed() {
    for key in [
        "command",
        "surface",
        "status",
        "kind",
        "feature",
        "check",
        "severity",
        "tool",
        "plugin",
        "http.route",
        "http.request.method",
        "http.response.status_code",
    ] {
        assert!(
            metric_label_is_allowed(key),
            "{key} is a declared metric label"
        );
    }
    for key in ["cli.install.id", "session.id", "user", "path", "url", "arg"] {
        assert!(
            !metric_label_is_allowed(key),
            "{key} would give a metric unbounded cardinality"
        );
    }
}

#[test]
fn retaining_leaves_the_surviving_values_untouched() {
    let mut attrs = vec![kv("command", "build"), kv("api_key", "sk-1")];
    rules(TelemetryLevel::Usage).retain_attributes(&mut attrs);
    assert_eq!(attrs.len(), 1);
    assert_eq!(attrs[0].value.as_str(), "build");
}
