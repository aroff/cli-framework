// tests/unit/telemetry_probe.rs
use cli_framework::telemetry::{
    feature_outcome, probe::effective, FeatureOutcome, ProbeIdError, ProbeRegistry, ProbeSpec,
    TelemetryLevel,
};

fn spec(id: &'static str, min_level: TelemetryLevel) -> ProbeSpec {
    ProbeSpec {
        id,
        min_level,
        summary: "s",
        sends: "nothing",
    }
}

fn all_enabled(_: &str) -> bool {
    true
}

#[test]
fn a_probe_id_may_be_dotted_lowercase_with_underscores_after_the_first_segment() {
    let mut r = ProbeRegistry::new();
    r.register(spec("cli.command.arg_values", TelemetryLevel::Debug))
        .unwrap();
    assert!(r.contains("cli.command.arg_values"));
}

#[test]
fn a_probe_id_with_an_uppercase_or_empty_segment_is_rejected() {
    let mut r = ProbeRegistry::new();
    assert!(matches!(
        r.register(spec("Cli.command", TelemetryLevel::Usage)),
        Err(ProbeIdError::Malformed(_))
    ));
    assert!(matches!(
        r.register(spec("cli..command", TelemetryLevel::Usage)),
        Err(ProbeIdError::Malformed(_))
    ));
    assert!(matches!(
        r.register(spec("cli_command", TelemetryLevel::Usage)),
        Err(ProbeIdError::Malformed(_))
    ));
}

#[test]
fn every_reserved_first_segment_is_refused() {
    for reserved in [
        "level",
        "attribution",
        "install_id",
        "notice_shown",
        "endpoint",
        "traces",
        "metrics",
        "logs",
    ] {
        let mut r = ProbeRegistry::new();
        let id: &'static str = Box::leak(format!("{reserved}.thing").into_boxed_str());
        assert!(
            matches!(
                r.register(spec(id, TelemetryLevel::Usage)),
                Err(ProbeIdError::Reserved(_, _))
            ),
            "'{id}' should have been refused"
        );
    }
}

#[test]
fn a_probe_may_not_shadow_the_enabled_switch_the_framework_owns() {
    // `telemetry.<probe>.enabled` is generated for every probe, so `a` and
    // `a.enabled` both claim `telemetry.a.enabled`. Refused at registration,
    // which is the earliest point and the only one with a useful message: the
    // manifest generator that would otherwise hit the collision has nowhere to
    // put a second field of a different kind under the same key.
    let mut r = ProbeRegistry::new();
    assert!(
        matches!(
            r.register(spec("cli.enabled", TelemetryLevel::Usage)),
            Err(ProbeIdError::ShadowsEnabledSwitch(_))
        ),
        "'cli.enabled' collides with probe 'cli''s own switch"
    );
    assert!(
        matches!(
            r.register(spec("cli.enabled.detail", TelemetryLevel::Usage)),
            Err(ProbeIdError::ShadowsEnabledSwitch(_))
        ),
        "the segment is refused wherever it appears after the first, not just last"
    );

    // The first segment is deliberately still allowed: there is no
    // `telemetry.enabled` key in this design, so `enabled.thing` collides with
    // nothing. Narrowing the rule to what actually collides keeps the error
    // honest.
    r.register(spec("enabled.thing", TelemetryLevel::Usage))
        .expect("'enabled' as a first segment shadows no framework key");

    // And the ordinary hierarchical case is untouched.
    r.register(spec("cli.command", TelemetryLevel::Usage))
        .expect("a plain child probe is fine");
    r.register(spec("cli.command.args", TelemetryLevel::Diagnostic))
        .expect("a grandchild probe is fine");
}

#[test]
fn registering_the_same_probe_id_twice_is_an_error() {
    let mut r = ProbeRegistry::new();
    r.register(spec("cli.command", TelemetryLevel::Usage))
        .unwrap();
    assert!(matches!(
        r.register(spec("cli.command", TelemetryLevel::Debug)),
        Err(ProbeIdError::Duplicate(_))
    ));
}

#[test]
fn a_probe_is_effective_only_at_or_above_its_minimum_telemetry_level() {
    let mut r = ProbeRegistry::new();
    r.register(spec("cli.command", TelemetryLevel::Usage))
        .unwrap();
    r.register(spec("cli.command.args", TelemetryLevel::Diagnostic))
        .unwrap();

    assert!(!effective(
        &r,
        TelemetryLevel::Off,
        "cli.command",
        &all_enabled
    ));
    assert!(effective(
        &r,
        TelemetryLevel::Usage,
        "cli.command",
        &all_enabled
    ));
    assert!(!effective(
        &r,
        TelemetryLevel::Usage,
        "cli.command.args",
        &all_enabled
    ));
    assert!(effective(
        &r,
        TelemetryLevel::Diagnostic,
        "cli.command.args",
        &all_enabled
    ));
}

#[test]
fn disabling_a_parent_probe_disables_its_whole_subtree() {
    let mut r = ProbeRegistry::new();
    r.register(spec("cli.command", TelemetryLevel::Usage))
        .unwrap();
    r.register(spec("cli.command.args", TelemetryLevel::Usage))
        .unwrap();
    let parent_off = |id: &str| id != "cli.command";

    assert!(!effective(
        &r,
        TelemetryLevel::Debug,
        "cli.command",
        &parent_off
    ));
    assert!(
        !effective(&r, TelemetryLevel::Debug, "cli.command.args", &parent_off),
        "a disabled parent must disable the child even though the child itself is enabled"
    );
}

#[test]
fn an_unregistered_probe_is_never_effective() {
    let r = ProbeRegistry::new();
    assert!(!effective(
        &r,
        TelemetryLevel::Debug,
        "cli.command",
        &all_enabled
    ));
}

#[test]
fn feature_outcome_labels_only_registered_feature_names() {
    let registered = ["export", "sync"];
    assert_eq!(
        feature_outcome(&registered, "export"),
        FeatureOutcome::Recorded
    );
    assert_eq!(
        feature_outcome(&registered, "user-typed-thing"),
        FeatureOutcome::Unregistered
    );
}
