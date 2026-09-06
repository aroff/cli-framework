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

use crate::config::resolution::Layer;
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
    /// The winning layer is `config_file`: this level is the person's
    /// own recorded choice, so announcing it would be telling them
    /// something they just said. Distinct from `AlreadyShown`, which
    /// means *we* told them; here nobody did, and nobody needs to.
    ConfiguredLocally,
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

    // Template 1 is "shown when the effective level is `off` and
    // `notice_shown` is empty". An unavailable store can never record a
    // level, so `None` is also what makes the spec's "printed every run"
    // case fall out here without a branch of its own.
    if policy.level == TelemetryLevel::Off {
        return match notice_shown {
            None => NoticeDecision::Show {
                text: notice_text(policy),
                announced_level: TelemetryLevel::Off,
            },
            Some(_) => NoticeDecision::Skip(SkipReason::AlreadyShown),
        };
    }

    // Template 2 requires "the winning layer is not `config_file`". Above
    // `off` with `config_file` winning, the level came from this person's
    // own settings file — they turned it on themselves.
    if policy.level_source == Layer::ConfigFile {
        return NoticeDecision::Skip(SkipReason::ConfiguredLocally);
    }

    // "above the stored `notice_shown`". Absent is below every level, so a
    // first run announces; equal or lower never does, which is the spec's
    // "a lowered level never triggers a notice" with no separate test.
    //
    // `matches!` rather than `Option::is_some_and`, which needs a newer
    // Rust than this crate's floor.
    if matches!(notice_shown, Some(shown) if policy.level <= shown) {
        return NoticeDecision::Skip(SkipReason::AlreadyShown);
    }

    NoticeDecision::Show {
        text: notice_text(policy),
        announced_level: policy.level,
    }
}

/// The notice body: two lines, the wording fixed by the spec.
///
/// The spec prints both templates as a single run of prose and calls them
/// "two lines"; the break is at the sentence boundary, which is also what
/// makes "a third clause `Details: <url>` is appended to the second line"
/// mean anything. `Details:` therefore joins line two with a space — a
/// third *line* would contradict the two-line rule directly above it.
fn notice_text(policy: &TelemetryPolicy) -> String {
    let app = &policy.app;
    let (first, second) = if policy.level == TelemetryLevel::Off {
        (
            format!("{app}: usage statistics are off."),
            format!(
                "Turn them on with `{app} telemetry set usage`; see what would be sent \
                 with `{app} telemetry info`."
            ),
        )
    } else {
        // No policy name means no policy client (see
        // `TelemetryInputs::policy_name`). Naming a policy we cannot read
        // would be an invention in a notice whose entire purpose is not
        // misleading people about what their machine is doing.
        let who = match &policy.policy_name {
            Some(name) => format!("policy {name:?}"),
            None => "an organisation policy".to_string(),
        };
        // Same reasoning: an anonymous Install carries no install id, so
        // the clause announcing one would be false on it.
        let tagged = if policy.install_id.is_some() {
            ", tagged with a random install id"
        } else {
            ""
        };
        (
            format!("{app}: {who} turned on {} telemetry{tagged}.", policy.level),
            format!(
                "Review with `{app} telemetry status`; opt out with \
                 `{app} telemetry set off`."
            ),
        )
    };
    let mut text = format!("{first}\n{second}");
    if let Deployment::EndUser {
        privacy_url: Some(url),
    } = &policy.deployment
    {
        text.push_str(&format!(" Details: {url}"));
    }
    text
}
