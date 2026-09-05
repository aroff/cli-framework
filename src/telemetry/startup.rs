//! The fixed order in which telemetry starts up.
//!
//! Every entry is later than the one above for a specific reason, recorded on
//! the variant. Naming the order and pinning it with a test is cheap; finding
//! out that the boundary saw a policy mid-mutation is not.

// `DoctorFinding` lives at `crate::doctor`, not re-exported into `telemetry`
// (confirmed by grep: `subscriber.rs`'s own `foreign_subscriber_finding`
// reaches it the same full-path way), so it is named directly here rather
// than via `super::`.
use super::{KillSwitch, StoreState, SubscriberOutcome};
use crate::doctor::DoctorFinding;

/// One step of telemetry startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupStep {
    /// Before any disk or socket access — `DO_NOT_TRACK=1` must cost nothing.
    KillSwitches,
    /// Needed by resolution; failure is a value, never an abort.
    OpenStore,
    /// The generated section must exist before resolution can name its leaves.
    MergeManifest,
    /// One pure resolution, once per process.
    Resolve,
    /// Frozen into an `Arc` and never mutated after this point.
    FreezePolicy,
    /// Providers and the export boundary; they hold the policy by `Arc`.
    BuildProviders,
    /// After the providers, because the OTel layer needs a live tracer.
    InstallSubscriber,
    /// After the subscriber so it can be logged; before dispatch so a person
    /// sees it before the command's own output.
    ShowNotice,
    /// Last of the setup: a panic during startup should be reported by
    /// whatever hook was already installed, not by a half-built one.
    InstallPanicHook,
    /// The command itself.
    Dispatch,
}

/// The order, as a value so it can be asserted against.
pub fn startup_order() -> &'static [StartupStep] {
    &[
        StartupStep::KillSwitches,
        StartupStep::OpenStore,
        StartupStep::MergeManifest,
        StartupStep::Resolve,
        StartupStep::FreezePolicy,
        StartupStep::BuildProviders,
        StartupStep::InstallSubscriber,
        StartupStep::ShowNotice,
        StartupStep::InstallPanicHook,
        StartupStep::Dispatch,
    ]
}

/// What startup observed, as opposed to what it decided. The decisions live
/// in `TelemetryPolicy`; this is the evidence a person needs when the
/// decisions are not the ones they expected.
///
/// `Default` matters: the doctor tests construct one with a single field set
/// and `..Default::default()` for the rest, so a field added here does not
/// touch ten tests. That requires `SubscriberOutcome: Default` too, with
/// `Installed` as the default — the ordinary case.
#[derive(Debug, Clone, Default)]
pub struct StartupReport {
    pub subscriber: SubscriberOutcome,
    pub store: StoreState,
    pub kill_switch: Option<KillSwitch>,
    /// `<APP>_TELEMETRY_*` variables that matched no manifest leaf (PR2
    /// Task 7). Names only — never values, which is the whole point of
    /// reporting them at all.
    pub unmatched_env: Vec<String>,
    pub findings: Vec<DoctorFinding>,
}
