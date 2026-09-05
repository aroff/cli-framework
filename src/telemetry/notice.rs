//! The first-run notice (Task 24).
//!
//! One pure function of four inputs. "Did we tell them" is the kind of
//! decision that accretes an `if` per release until nobody can say when it
//! fires, so it gets a total function and a test per branch instead of a
//! condition spread across startup.
//!
//! **It goes to stderr, not stdout.** A notice on stdout corrupts
//! `myapp list --json | jq`, which turns a privacy notice into a bug report.
//! Writing to stderr and checking `stderr_is_tty` is the caller's job (this
//! module is pure); `notice_decision` only decides *whether* and *what*.
//!
//! `notice_shown` stores the level that was announced, not a boolean. If a
//! later release raises the default from `off` to `usage`, an install that
//! was told about `off` has not been told about `usage`, and the notice
//! fires again. A boolean would silently swallow that.
//!
//! The caller writes `telemetry.notice_shown` **after** printing, and a
//! write failure is a warning, not an error: re-showing a notice is a small
//! annoyance, failing to start because the notice bookkeeping failed is not.

use crate::telemetry::axes::{Deployment, TelemetryLevel};
use crate::telemetry::policy::TelemetryPolicy;

/// The interface a command was invoked through.
///
/// This is Task 17's type (`src/telemetry/probes.rs`, PR5 — "the root span
/// and the command probe"), not this PR's. PR5 has not landed yet, so this
/// module — which legitimately belongs to this PR (Task 24) — cannot
/// compile without *some* definition of `Surface`. A minimal stand-in is
/// defined here instead, matching the plan's Task 17 shape exactly (same
/// derives, same four variants, same `as_str` mapping), so that when PR5
/// merges `probes.rs`'s real definition, the conflict is a duplicate `enum
/// Surface` the reviewing session resolves by deleting this copy and
/// re-exporting PR5's from here instead. See the PR6 final report for the
/// full disclosure of this deviation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Cli,
    Chat,
    Mcp,
    Api,
}

impl Surface {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Chat => "chat",
            Self::Mcp => "mcp",
            Self::Api => "api",
        }
    }
}

/// Why no notice was shown. Every variant is a deliberate case, not a
/// fall-through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    AlreadyShown,
    NotInteractive,
    NotAHumanSurface,
    ServiceDeployment,
    KillSwitch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeDecision {
    Show {
        text: String,
        /// Stored in `telemetry.notice_shown`. A level, not a boolean: an
        /// install told about `off` has not been told about `usage`, so a
        /// raised default re-notifies rather than silently starting to
        /// send.
        announced_level: TelemetryLevel,
    },
    Skip(SkipReason),
}

pub fn notice_decision(
    policy: &TelemetryPolicy,
    notice_shown: Option<TelemetryLevel>,
    surface: Surface,
    stderr_is_tty: bool,
) -> NoticeDecision {
    if matches!(policy.deployment, Deployment::Service) {
        return NoticeDecision::Skip(SkipReason::ServiceDeployment);
    }
    if policy.kill_switch.is_some() {
        // Someone who set DO_NOT_TRACK has already expressed the preference
        // the notice exists to ask about.
        return NoticeDecision::Skip(SkipReason::KillSwitch);
    }
    if !matches!(surface, Surface::Cli | Surface::Chat) {
        return NoticeDecision::Skip(SkipReason::NotAHumanSurface);
    }
    if !stderr_is_tty {
        return NoticeDecision::Skip(SkipReason::NotInteractive);
    }
    if notice_shown == Some(policy.level) {
        return NoticeDecision::Skip(SkipReason::AlreadyShown);
    }
    NoticeDecision::Show {
        text: notice_text(policy),
        announced_level: policy.level,
    }
}

fn notice_text(policy: &TelemetryPolicy) -> String {
    let mut text = if policy.level == TelemetryLevel::Off {
        format!(
            "{}: telemetry is off. Run `{} telemetry set usage` to help improve it.",
            policy.app, policy.app
        )
    } else {
        format!(
            "{}: anonymous usage telemetry is on. Run `{} telemetry status` to see what is sent, \
             or `{} telemetry set off` to turn it off.",
            policy.app, policy.app, policy.app
        )
    };
    if let Deployment::EndUser {
        privacy_url: Some(url),
    } = &policy.deployment
    {
        text.push_str(&format!("\nDetails: {url}"));
    }
    text
}
