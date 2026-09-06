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

// ---------------------------------------------------------------------------
// Exact-text and ordering tests.
//
// Everything above asserts `text.contains("telemetry")` or similar. That
// oracle passes on any wording that happens to mention the word, which is how
// a paraphrase of both templates reached review. The spec fixes the wording
// ("Two templates, English, two lines, no override hook"), so the tests below
// compare the whole string. When a template legitimately changes, one
// assertion fails and names the new text — which is the point.
// ---------------------------------------------------------------------------

/// The spec's template 1, with `myapp` substituted by the fixture's app name.
const OFF_TEMPLATE: &str = "demo: usage statistics are off.\n\
     Turn them on with `demo telemetry set usage`; see what would be sent with \
     `demo telemetry info`.";

/// The spec's template 2, likewise.
const ON_TEMPLATE: &str = "demo: policy \"Acme default\" turned on diagnostic telemetry, \
     tagged with a random install id.\n\
     Review with `demo telemetry status`; opt out with `demo telemetry set off`.";

fn shown_text(policy: &cli_framework::telemetry::TelemetryPolicy) -> String {
    match decision(policy, None, Surface::Cli, true) {
        NoticeDecision::Show { text, .. } => text,
        other => panic!("expected a notice, got {other:?}"),
    }
}

#[test]
fn the_off_notice_is_word_for_word_the_template_the_spec_wrote() {
    assert_eq!(shown_text(&enduser(TelemetryLevel::Off)), OFF_TEMPLATE);
}

#[test]
fn the_on_notice_is_word_for_word_the_template_the_spec_wrote() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Diagnostic,
        |p| p.policy_name = Some("Acme default".to_string()),
    );
    assert_eq!(shown_text(&policy), ON_TEMPLATE);
}

#[test]
fn without_a_policy_client_the_notice_does_not_invent_a_policy_name() {
    // `policy_name: None` means no policy client at all. The notice still has
    // to say *something* turned telemetry on, but naming a policy nobody
    // configured would be a fabrication in the one message whose entire job is
    // telling a person the truth about their machine.
    let text = shown_text(&enduser(TelemetryLevel::Diagnostic));
    assert!(
        text.starts_with("demo: an organisation policy turned on diagnostic telemetry"),
        "{text}"
    );
    assert!(
        !text.contains("policy \"\""),
        "an empty quoted name is worse than no name: {text}"
    );
}

#[test]
fn an_anonymous_install_is_not_told_it_carries_an_install_id() {
    // Attribution `anonymous` drops the install id in `resolve_policy`. The
    // "tagged with a random install id" clause would then be a false statement
    // about what is being sent.
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Diagnostic,
        |p| {
            p.install_id = None;
            p.policy_name = Some("Acme default".to_string());
        },
    );
    let text = shown_text(&policy);
    assert!(
        !text.contains("install id"),
        "there is no install id to tag anything with: {text}"
    );
    assert!(
        text.starts_with("demo: policy \"Acme default\" turned on diagnostic telemetry."),
        "{text}"
    );
}

#[test]
fn a_level_the_person_set_themselves_is_never_announced() {
    // Spec: template 2 is shown only when "the winning layer is not
    // `config_file`". A level read back out of the person's own settings file
    // is their own decision; announcing it tells them what they just said.
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Diagnostic,
        |p| p.level_source = cli_framework::config::resolution::Layer::ConfigFile,
    );
    assert!(
        matches!(
            decision(&policy, None, Surface::Cli, true),
            NoticeDecision::Skip(SkipReason::ConfiguredLocally)
        ),
        "got {:?}",
        decision(&policy, None, Surface::Cli, true)
    );
}

#[test]
fn a_lowered_level_never_triggers_a_notice() {
    // Spec: shown when the level is *above* the stored `notice_shown`, and "a
    // lowered level never triggers a notice". An equality test reads a drop
    // from diagnostic to usage as "different, so announce", which announces a
    // reduction in what is sent as though it were an increase.
    assert!(
        matches!(
            decision(
                &enduser(TelemetryLevel::Usage),
                Some(TelemetryLevel::Diagnostic),
                Surface::Cli,
                true,
            ),
            NoticeDecision::Skip(SkipReason::AlreadyShown)
        ),
        "got {:?}",
        decision(
            &enduser(TelemetryLevel::Usage),
            Some(TelemetryLevel::Diagnostic),
            Surface::Cli,
            true,
        )
    );
}

#[test]
fn an_install_already_told_about_off_is_not_told_about_off_again() {
    // Template 1 is shown "when the effective level is `off` and
    // `notice_shown` is empty" — empty, not merely different. `Some(usage)`
    // recorded before a drop back to `off` is not empty.
    assert!(matches!(
        decision(
            &enduser(TelemetryLevel::Off),
            Some(TelemetryLevel::Usage),
            Surface::Cli,
            true
        ),
        NoticeDecision::Skip(SkipReason::AlreadyShown)
    ));
}

#[test]
fn the_details_clause_joins_the_second_line_rather_than_making_a_third() {
    // Spec: "a third clause `Details: <url>` is appended to the second line",
    // two sentences after "two lines". A third line satisfies neither.
    let policy = policy_with(
        Deployment::EndUser {
            privacy_url: Some("https://example.com/privacy".into()),
        },
        TelemetryLevel::Off,
        |_| {},
    );
    let text = shown_text(&policy);
    assert_eq!(
        text,
        format!("{OFF_TEMPLATE} Details: https://example.com/privacy")
    );
    assert_eq!(text.lines().count(), 2, "{text}");
}
