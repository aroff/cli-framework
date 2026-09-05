// tests/unit/telemetry_redact.rs
use cli_framework::telemetry::{
    attribute_min_level, is_never_listed, metric_label_is_allowed, probe_of, Deployment, KeyValue,
    RedactionRules, TelemetryLevel, NEVER_KEYS, NEVER_LIST_EXEMPT, PROBE_ATTR_KEY,
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
fn a_probe_attribute_that_is_not_a_string_names_no_probe() {
    // The boundary keys every decision off the probe id, so `probe_of` is the
    // one place where "there is no probe here" gets decided. An instrumentation
    // site that set `cli.probe` to a number is not naming a probe — the value
    // has no id grammar, cannot match a `ProbeSpec`, and must therefore leave
    // the span unprobed so `span_verdict` drops it rather than waving it
    // through under a probe that does not exist.
    let numeric = KeyValue::new(PROBE_ATTR_KEY, 7i64);
    assert_eq!(probe_of(&[numeric]), None);

    let boolean = KeyValue::new(PROBE_ATTR_KEY, true);
    assert_eq!(probe_of(&[boolean]), None);

    // And the string case still resolves through the very same call, so the
    // two assertions above are about the value's type and nothing else.
    assert_eq!(
        probe_of(&[KeyValue::new(PROBE_ATTR_KEY, "cli.command")]),
        Some("cli.command")
    );
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

#[test]
fn the_usage_error_token_survives_at_debug_and_is_dropped_below_it() {
    // `cli.usage_error.token` is the one framework key the built-in never-list
    // would otherwise swallow whole: it contains the substring `token`, the
    // never-list is checked first and returns immediately, so before the
    // exemption existed this attribute could never be emitted at any level and
    // its entry in the level table was dead code that no test noticed.
    //
    // The specification puts it on the usage-error event at `debug`, and says
    // the boundary "clears `exception.message` and `cli.usage_error.token`
    // below `debug`" — a sentence that only means anything if the key is kept
    // *at* `debug`.
    assert!(
        !is_never_listed("cli.usage_error.token", &[]),
        "the framework's own reviewed key must not be caught by the *token* glob"
    );
    assert!(
        rules(TelemetryLevel::Debug).keeps_attribute("cli.usage_error.token"),
        "at debug the offending token is exactly what makes a usage-error report actionable"
    );
    for level in [
        TelemetryLevel::Off,
        TelemetryLevel::Usage,
        TelemetryLevel::Diagnostic,
    ] {
        assert!(
            !rules(level).keeps_attribute("cli.usage_error.token"),
            "below debug the token is a fragment of the user's command line, so {level:?} \
             must drop it"
        );
    }
}

#[test]
fn the_exemption_is_exact_and_does_not_widen_the_token_glob() {
    // The exemption is a list of whole keys, not a prefix or a substring. If it
    // were loose, every one of these would ride in behind it — which is the
    // failure mode that makes carve-outs in a redaction rule dangerous.
    for key in [
        "cli.usage_error.token.value",
        "cli.usage_error.tokens",
        "xcli.usage_error.token",
        "CLI.USAGE_ERROR.TOKEN",
        "usage_error.token",
        "token",
    ] {
        assert!(
            is_never_listed(key, &[]),
            "{key} is not the exempted key and must still be dropped"
        );
    }
}

#[test]
fn an_author_who_extends_the_never_list_outranks_the_exemption() {
    // Extending the never-list is a deliberate decision by someone who has
    // concluded their product cannot carry a class of value. This crate's
    // judgement about its own key does not survive that.
    let extra = vec!["token".to_string()];
    assert!(
        is_never_listed("cli.usage_error.token", &extra),
        "an author's own never-list entry must win over the framework exemption"
    );
    let narrower = vec!["cli.usage_error".to_string()];
    assert!(
        is_never_listed("cli.usage_error.token", &narrower),
        "the author's entries are substrings, so a prefix they added catches it too"
    );
}

#[test]
fn an_empty_never_list_entry_does_not_drop_every_attribute() {
    // `"".contains("")` is true, and so is `anything.contains("")`. One stray
    // empty string in `with_telemetry_never` — a trailing comma in a config
    // list, a variable that resolved to nothing — used to strip every
    // attribute from every span in the product, leaving telemetry that looks
    // alive and carries nothing at all. Blanks are ignored instead.
    for blank in ["", " ", "\t", "\n  "] {
        let extra = vec![blank.to_string()];
        assert!(
            !is_never_listed("cli.command", &extra),
            "a blank never-list entry ({blank:?}) must not match every key"
        );
        let rules = RedactionRules {
            level: TelemetryLevel::Usage,
            app_attr_allowlist: Vec::new(),
            extra_never: vec![blank.to_string()],
        };
        assert!(
            rules.keeps_attribute("cli.command"),
            "and it must not reach the boundary either ({blank:?})"
        );
    }

    // A real entry alongside a blank still works — ignoring blanks must not
    // turn into ignoring the list.
    let mixed = vec![String::new(), "internal_id".to_string()];
    assert!(is_never_listed("app.internal_id", &mixed));
    assert!(!is_never_listed("cli.command", &mixed));
}

#[test]
fn keys_the_specification_forbids_at_any_level_are_dropped_at_every_level() {
    // The specification's never-at-any-level list names raw URLs and paths,
    // host names and command lines, and says the redacting exporter is the only
    // place these rules live. Without this the framework prefix `url.` would
    // have carried `url.full` at plain `usage`, on the strength of nothing more
    // than "no instrumentation site sets it *today*".
    for key in NEVER_KEYS {
        assert!(
            is_never_listed(key, &[]),
            "{key} is forbidden at any telemetry level"
        );
        for level in [
            TelemetryLevel::Usage,
            TelemetryLevel::Diagnostic,
            TelemetryLevel::Debug,
        ] {
            assert!(
                !rules(level).keeps_attribute(key),
                "{key} must not survive the boundary at {level:?}"
            );
        }
        // Not even by being allowlisted: the never-list wins over the allowlist.
        let permissive = RedactionRules {
            level: TelemetryLevel::Debug,
            app_attr_allowlist: vec![key.to_string()],
            extra_never: Vec::new(),
        };
        assert!(
            !permissive.keeps_attribute(key),
            "{key} must not be reachable by adding it to the app allowlist"
        );
    }

    // The bounded alternative stays: a matched route template is a fixed set of
    // strings the author wrote, not user data.
    assert!(rules(TelemetryLevel::Usage).keeps_attribute("http.route"));
    // And the address the specification does permit, at diagnostic, is not
    // caught by the host-name rule.
    assert!(rules(TelemetryLevel::Diagnostic).keeps_attribute("server.address"));
}

#[test]
fn the_never_key_and_exemption_lists_stay_disjoint_and_lower_case() {
    // A key on both lists would make the outcome depend on evaluation order,
    // which is exactly the kind of thing that survives review and then decides
    // whether a credential ships.
    for exempt in NEVER_LIST_EXEMPT {
        assert!(
            !NEVER_KEYS.contains(exempt),
            "{exempt} cannot be both forbidden outright and exempt"
        );
    }
    // `is_never_listed` lower-cases the key before comparing against
    // NEVER_KEYS, so an upper-case entry there would silently never match.
    for key in NEVER_KEYS {
        assert_eq!(
            *key,
            key.to_ascii_lowercase(),
            "NEVER_KEYS entries are compared against a lower-cased key"
        );
    }
}
