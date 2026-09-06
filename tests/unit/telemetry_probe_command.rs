// tests/unit/telemetry_probe_command.rs
use cli_framework::telemetry::{
    command_metric_labels, command_span_attrs, CommandOutcome, CommandStatus, Deployment, Surface,
    TelemetryLevel,
};

mod support;
use support::policy_with;

fn labels(outcome: &CommandOutcome) -> Vec<(String, String)> {
    command_metric_labels(outcome)
        .into_iter()
        .map(|kv| (kv.key.to_string(), kv.value.as_str().to_string()))
        .collect()
}

fn outcome(command: Option<&str>, status: CommandStatus) -> CommandOutcome {
    CommandOutcome {
        command: command.map(|c| c.to_string()),
        surface: Surface::Cli,
        status,
        duration_ms: 12.5,
    }
}

#[test]
fn a_registered_command_becomes_a_label() {
    let l = labels(&outcome(Some("config get"), CommandStatus::Ok));
    assert!(l.contains(&("command".to_string(), "config get".to_string())));
    assert!(l.contains(&("surface".to_string(), "cli".to_string())));
    assert!(l.contains(&("status".to_string(), "ok".to_string())));
}

#[test]
fn an_unregistered_command_contributes_no_command_label_at_all() {
    let l = labels(&outcome(None, CommandStatus::UsageError));
    assert!(
        !l.iter().any(|(k, _)| k == "command"),
        "a label taken from unvalidated input is unbounded cardinality and a \
         leak — a mistyped command is often a mistyped path or a pasted token: {l:?}"
    );
    assert!(l.contains(&("status".to_string(), "usage_error".to_string())));
}

#[test]
fn every_metric_label_is_on_the_closed_allowlist() {
    use cli_framework::telemetry::metric_label_is_allowed;
    for status in [
        CommandStatus::Ok,
        CommandStatus::UsageError,
        CommandStatus::Error,
    ] {
        for (k, _) in labels(&outcome(Some("build"), status)) {
            assert!(
                metric_label_is_allowed(&k),
                "{k} is not an allowed metric label"
            );
        }
    }
}

#[test]
fn the_root_span_carries_the_installation_and_the_session() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.install_id = Some("install-9".into());
            p.session_id = "session-9".into();
        },
    );
    let attrs = command_span_attrs(&policy, &outcome(Some("build"), CommandStatus::Ok));
    let map: Vec<(String, String)> = attrs
        .into_iter()
        .map(|kv| (kv.key.to_string(), kv.value.as_str().to_string()))
        .collect();
    assert!(map.contains(&("cli.install.id".to_string(), "install-9".to_string())));
    assert!(map.contains(&("session.id".to_string(), "session-9".to_string())));
    assert!(map.contains(&("cli.telemetry.level".to_string(), "usage".to_string())));
    assert!(map.contains(&("cli.probe".to_string(), "cli.command".to_string())));
}

#[test]
fn an_anonymous_install_puts_no_install_id_on_the_span() {
    use cli_framework::telemetry::Attribution;
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.attribution = Attribution::Anonymous;
            p.install_id = None;
        },
    );
    let attrs = command_span_attrs(&policy, &outcome(Some("build"), CommandStatus::Ok));
    assert!(!attrs.iter().any(|kv| kv.key.as_str() == "cli.install.id"));
}

#[test]
fn the_duration_is_recorded_in_milliseconds_as_a_float() {
    let o = outcome(Some("build"), CommandStatus::Ok);
    assert_eq!(
        o.duration_ms, 12.5,
        "sub-millisecond commands are common; an integer would floor them to zero"
    );
}

#[test]
fn each_surface_has_a_stable_lowercase_label() {
    for (surface, expected) in [
        (Surface::Cli, "cli"),
        (Surface::Chat, "chat"),
        (Surface::Mcp, "mcp"),
        (Surface::Api, "api"),
    ] {
        assert_eq!(surface.as_str(), expected);
    }
}

// Not in the plan's own test list — added because `execute_command_direct`
// (src/app/builder.rs) receives an `InvocationSurface` from the dispatch
// layer and must turn it into the probe catalog's own `Surface` to build a
// `CommandOutcome`. `Cli` is the only variant either builder.rs callsite
// constructs today (chat/mcp/api dispatch is out of this task's scope), so
// without this test the other three arms of the `From` impl have no
// coverage at all.
#[test]
fn every_invocation_surface_maps_onto_the_matching_probe_surface() {
    use cli_framework::app::dispatch::InvocationSurface;
    for (invocation, expected) in [
        (InvocationSurface::Cli, Surface::Cli),
        (InvocationSurface::Chat, Surface::Chat),
        (InvocationSurface::Mcp, Surface::Mcp),
        (InvocationSurface::Api, Surface::Api),
    ] {
        assert_eq!(Surface::from(invocation), expected);
    }
}
