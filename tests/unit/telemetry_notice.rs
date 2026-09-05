// tests/unit/telemetry_notice.rs
//! Unit tests for the first-run notice decision (Task 24).
//!
//! One fixup relative to the plan text, required by the real struct
//! definition in `src/telemetry/policy.rs`: `TelemetryPolicy::kill_switch` is
//! `Option<KillSwitch>`, not `Option<String>`, so
//! `a_kill_switch_suppresses_the_notice_too` sets
//! `p.kill_switch = Some(KillSwitch::DoNotTrack)` rather than a bare string
//! (the same fixup already applied in `tests/unit/telemetry_commands.rs`).

use cli_framework::telemetry::{
    notice_decision, Deployment, KillSwitch, NoticeDecision, SkipReason, Surface, TelemetryLevel,
};

mod support;
use support::policy_with;

fn enduser(level: TelemetryLevel) -> cli_framework::telemetry::TelemetryPolicy {
    policy_with(Deployment::EndUser { privacy_url: None }, level, |_| {})
}

fn decision(
    policy: &cli_framework::telemetry::TelemetryPolicy,
    shown: Option<TelemetryLevel>,
    surface: Surface,
    tty: bool,
) -> NoticeDecision {
    notice_decision(policy, shown, surface, tty)
}

#[test]
fn a_first_run_on_an_interactive_terminal_shows_the_notice() {
    match decision(&enduser(TelemetryLevel::Usage), None, Surface::Cli, true) {
        NoticeDecision::Show {
            announced_level,
            text,
        } => {
            assert_eq!(announced_level, TelemetryLevel::Usage);
            assert!(text.contains("telemetry"));
        }
        other => panic!("expected a notice, got {other:?}"),
    }
}

#[test]
fn the_second_run_shows_nothing() {
    assert!(matches!(
        decision(
            &enduser(TelemetryLevel::Usage),
            Some(TelemetryLevel::Usage),
            Surface::Cli,
            true
        ),
        NoticeDecision::Skip(SkipReason::AlreadyShown)
    ));
}

#[test]
fn an_install_told_about_off_is_told_again_when_the_default_rises() {
    match decision(
        &enduser(TelemetryLevel::Usage),
        Some(TelemetryLevel::Off),
        Surface::Cli,
        true,
    ) {
        NoticeDecision::Show {
            announced_level, ..
        } => {
            assert_eq!(
                announced_level,
                TelemetryLevel::Usage,
                "storing a boolean would silently swallow a raised default"
            );
        }
        other => panic!("a person told about off has not been told about usage: {other:?}"),
    }
}

#[test]
fn a_pipe_gets_no_notice_because_there_is_nobody_reading() {
    assert!(matches!(
        decision(&enduser(TelemetryLevel::Usage), None, Surface::Cli, false),
        NoticeDecision::Skip(SkipReason::NotInteractive)
    ));
}

#[test]
fn a_machine_surface_never_gets_a_notice_even_on_a_tty() {
    for surface in [Surface::Mcp, Surface::Api] {
        assert!(
            matches!(
                decision(&enduser(TelemetryLevel::Usage), None, surface, true),
                NoticeDecision::Skip(SkipReason::NotAHumanSurface)
            ),
            "a protocol stream is not a person; a notice there is corruption"
        );
    }
}

#[test]
fn the_chat_surface_does_get_a_notice_because_a_person_is_there() {
    assert!(matches!(
        decision(&enduser(TelemetryLevel::Usage), None, Surface::Chat, true),
        NoticeDecision::Show { .. }
    ));
}

#[test]
fn a_service_deployment_never_shows_a_notice() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Diagnostic, |_| {});
    assert!(matches!(
        decision(&policy, None, Surface::Cli, true),
        NoticeDecision::Skip(SkipReason::ServiceDeployment)
    ));
}

#[test]
fn a_kill_switch_suppresses_the_notice_too() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Off,
        |p| {
            p.kill_switch = Some(KillSwitch::DoNotTrack);
        },
    );
    assert!(
        matches!(
            decision(&policy, None, Surface::Cli, true),
            NoticeDecision::Skip(SkipReason::KillSwitch)
        ),
        "someone who set DO_NOT_TRACK has already expressed the preference the \
         notice exists to ask about"
    );
}

#[test]
fn the_off_notice_and_the_on_notice_are_different_text() {
    let off = decision(&enduser(TelemetryLevel::Off), None, Surface::Cli, true);
    let on = decision(&enduser(TelemetryLevel::Usage), None, Surface::Cli, true);
    let (off_text, on_text) = match (off, on) {
        (NoticeDecision::Show { text: a, .. }, NoticeDecision::Show { text: b, .. }) => (a, b),
        other => panic!("both should show: {other:?}"),
    };
    assert_ne!(off_text, on_text);
    assert!(
        on_text.contains("telemetry status"),
        "the on-notice says how to look"
    );
    assert!(
        off_text.contains("telemetry set"),
        "the off-notice says how to opt in"
    );
}

#[test]
fn a_privacy_url_appears_as_a_details_line_and_nothing_else_changes() {
    let without = enduser(TelemetryLevel::Usage);
    let with = policy_with(
        Deployment::EndUser {
            privacy_url: Some("https://example.com/privacy".into()),
        },
        TelemetryLevel::Usage,
        |_| {},
    );
    let a = match decision(&without, None, Surface::Cli, true) {
        NoticeDecision::Show { text, .. } => text,
        other => panic!("{other:?}"),
    };
    let b = match decision(&with, None, Surface::Cli, true) {
        NoticeDecision::Show { text, .. } => text,
        other => panic!("{other:?}"),
    };
    assert!(
        b.starts_with(&a),
        "the details line is appended, not a rewrite"
    );
    assert!(b.contains("Details: https://example.com/privacy"));
}

#[test]
fn the_notice_is_one_line_plus_at_most_the_details_line() {
    let policy = policy_with(
        Deployment::EndUser {
            privacy_url: Some("https://example.com/p".into()),
        },
        TelemetryLevel::Usage,
        |_| {},
    );
    let text = match decision(&policy, None, Surface::Cli, true) {
        NoticeDecision::Show { text, .. } => text,
        other => panic!("{other:?}"),
    };
    assert!(
        text.lines().count() <= 2,
        "a notice a person scrolls past is a notice a person did not read: {text}"
    );
}

#[test]
fn each_surface_reports_its_own_wire_name() {
    assert_eq!(Surface::Cli.as_str(), "cli");
    assert_eq!(Surface::Chat.as_str(), "chat");
    assert_eq!(Surface::Mcp.as_str(), "mcp");
    assert_eq!(Surface::Api.as_str(), "api");
}
