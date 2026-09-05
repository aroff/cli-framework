//! Composing the one `tracing` subscriber a process gets (ADR 0078).
//!
//! `tracing`'s global dispatcher is set once and never replaced. Two things in
//! this framework want to set it — `init_default_logging` and telemetry
//! startup — and an application's own `main` may well have set it before
//! either. Rather than racing, this module makes the composition explicit and
//! gives the loser a defined outcome the caller can report.
//!
//! This module is reachable under the weaker `observability` feature (for
//! [`install_default_logging`]/[`LoggingGuard`], which owe nothing to
//! telemetry) as well as under `telemetry` (for the rest: composing the OTel
//! bridge layer and reporting a foreign subscriber as a doctor finding rather
//! than a startup failure).

/// What happened when we tried to install the subscriber.
#[cfg(feature = "telemetry")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubscriberOutcome {
    /// We installed it; spans reach the OTel layer.
    #[default]
    Installed,
    /// Something else got there first. Traces and logs are not exported;
    /// metrics are unaffected.
    ForeignSubscriber,
}

/// Emit the foreign-subscriber warning at most once per process.
///
/// Once, because the condition is permanent: a repeat on every span would be
/// noisier than the problem it reports, and would be the first thing an
/// operator silences.
#[cfg(feature = "telemetry")]
pub fn warn_once_foreign_subscriber(emit: &dyn Fn(&str)) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        emit(
            "telemetry: another tracing subscriber is already installed, so traces and \
             logs will not be exported. Metrics are unaffected. Run the doctor command \
             for details.",
        );
    });
}

/// The doctor's account of the same condition.
#[cfg(feature = "telemetry")]
pub fn foreign_subscriber_finding() -> crate::doctor::DoctorFinding {
    use crate::doctor::CheckSeverity;

    crate::doctor::DoctorFinding {
        check_id: "telemetry.subscriber".to_string(),
        title: "Tracing subscriber".to_string(),
        // Warning, not Error: installing your own subscriber is a legitimate
        // thing for an application to do, and the program works.
        severity: CheckSeverity::Warning,
        message: "Another tracing subscriber is installed; traces and logs are not exported."
            .to_string(),
        detail: Some(
            "A tracing subscriber can only be installed once per process, and one was \
             already in place when telemetry started. Metric export is unaffected \
             because it does not go through tracing."
                .to_string(),
        ),
        remediation: Some(
            "Remove the application's own subscriber installation, or call \
             cli_framework::init_default_logging() before building the application so \
             the framework composes both."
                .to_string(),
        ),
    }
}

#[cfg(feature = "telemetry")]
mod compose {
    use super::SubscriberOutcome;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;

    /// The filter both entry points use: `RUST_LOG` when set, `info` otherwise.
    pub fn filter() -> EnvFilter {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    }

    /// The implicit install: registry + filter + OTel layer.
    ///
    /// Deliberately no `fmt` layer. An application that wanted console output
    /// would have called `init_default_logging`; adding a second stderr writer
    /// to a program that never asked for one is a visible behaviour change
    /// dressed up as telemetry.
    ///
    /// `otel` must implement `Layer` for the subscriber stack as it exists
    /// *after* the filter is applied (`Layered<EnvFilter, Registry>`), not for
    /// the bare `Registry` — the filter is added first, so that is the actual
    /// type `.with(otel)` layers on top of.
    pub fn install_with_otel<L>(otel: L) -> SubscriberOutcome
    where
        L: tracing_subscriber::Layer<
                tracing_subscriber::layer::Layered<EnvFilter, tracing_subscriber::Registry>,
            > + Send
            + Sync
            + 'static,
    {
        let installed = tracing_subscriber::registry()
            .with(filter())
            .with(otel)
            .try_init()
            .is_ok();
        if installed {
            SubscriberOutcome::Installed
        } else {
            SubscriberOutcome::ForeignSubscriber
        }
    }
}

/// Compose the OTel bridge layer from `guard` and try to become the process
/// subscriber. This is telemetry startup's implicit install path (rule 3:
/// registry + filter + OTel layer, no `fmt` layer).
#[cfg(feature = "telemetry")]
pub fn install_telemetry_subscriber(guard: &crate::telemetry::TelemetryGuard) -> SubscriberOutcome {
    compose::install_with_otel(crate::telemetry::init::otel_layer(guard))
}

/// Test-only entry point: composes the same layers as
/// [`install_telemetry_subscriber`] but with a no-op OTel layer, so callers
/// need no provider. Calls the same `try_init` path as the real one — a
/// reimplementation here would prove nothing about the real composition.
#[doc(hidden)]
#[cfg(feature = "telemetry")]
pub fn install_subscriber_for_test() -> SubscriberOutcome {
    compose::install_with_otel(tracing_subscriber::layer::Identity::new())
}

/// A boxed `tracing` layer over the base `Registry`: the shape
/// [`LoggingGuard::attach_otel_layer`] takes, and what a caller boxes
/// [`crate::telemetry::init::otel_layer`] into to pass there.
#[cfg(feature = "telemetry")]
pub type BoxedLayer =
    Box<dyn tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync>;

/// The reload slot [`LoggingGuard`] carries when its install succeeded.
///
/// Wraps a handle over an initially-empty, optional, boxed layer slot layered
/// into the subscriber [`install_default_logging`] installs. Telemetry
/// startup can later `reload` it to `Some(otel_layer)` to attach OTel export
/// to a subscriber an application's own `main` already installed — the
/// mechanism, not yet the wiring: PR7 is where startup actually calls it.
#[cfg(feature = "telemetry")]
struct ReloadSlot(
    tracing_subscriber::reload::Handle<Option<BoxedLayer>, tracing_subscriber::Registry>,
);

/// Returned by [`crate::init_default_logging`].
///
/// It carries the reload handle that telemetry startup uses to add the OTel
/// layer to an already-installed subscriber, which is how an application can
/// call `init_default_logging()` in `main` and still get exported traces.
/// Holding it is not required for logging to work; dropping it only gives up
/// that upgrade path.
#[must_use = "hold the guard to let telemetry attach its layer later"]
pub struct LoggingGuard {
    #[cfg(feature = "telemetry")]
    reload: Option<ReloadSlot>,
}

impl LoggingGuard {
    /// Whether telemetry startup can still attach the OTel layer to this
    /// process's subscriber. `false` when [`install_default_logging`] lost
    /// the process global to something else, in which case there is no slot
    /// left to attach to (the foreign subscriber owns the composition).
    ///
    /// Under the weaker `observability`-only build there is no reload slot at
    /// all — `init_default_logging` cannot be upgraded by telemetry that was
    /// never compiled in — so this is always `false`.
    pub fn can_attach_otel_layer(&self) -> bool {
        #[cfg(feature = "telemetry")]
        {
            self.reload.is_some()
        }
        #[cfg(not(feature = "telemetry"))]
        {
            false
        }
    }

    /// Attach `layer` to this process's subscriber, replacing whatever the
    /// reload slot currently holds.
    ///
    /// A no-op, not an error, when
    /// [`can_attach_otel_layer`](Self::can_attach_otel_layer) is `false` —
    /// there is no slot to attach to (a foreign subscriber won the install),
    /// and this module's rule throughout is to degrade rather than fail when
    /// that happens.
    #[cfg(feature = "telemetry")]
    pub fn attach_otel_layer(
        &self,
        layer: BoxedLayer,
    ) -> Result<(), tracing_subscriber::reload::Error> {
        match &self.reload {
            Some(slot) => slot.0.reload(Some(layer)),
            None => Ok(()),
        }
    }
}

/// The new body of `crate::init_default_logging`: install a process-wide
/// subscriber and hand back a guard.
///
/// Rule 1: an application that never asks for telemetry gets what it has
/// today. Under `telemetry`, the subscriber additionally carries a reload
/// slot so telemetry startup can attach the OTel layer later without a
/// second, competing `try_init` call.
#[cfg(feature = "telemetry")]
pub fn install_default_logging() -> LoggingGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (reload_layer, handle) = tracing_subscriber::reload::Layer::new(None::<BoxedLayer>);

    // `reload_layer` must be the first layer added on top of the bare
    // `Registry`: `reload::Layer<L, S>` only implements `Layer<S>` for the
    // exact `S` it was constructed with (here, `Registry`), not for whatever
    // stack happens to be built by the time `.with()` reaches it.
    let installed = tracing_subscriber::registry()
        .with(reload_layer)
        .with(compose::filter())
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .try_init()
        .is_ok();

    LoggingGuard {
        reload: if installed {
            Some(ReloadSlot(handle))
        } else {
            None
        },
    }
}

/// The new body of `crate::init_default_logging`, `observability`-only build:
/// identical to the subscriber this crate installed before this PR.
#[cfg(not(feature = "telemetry"))]
pub fn install_default_logging() -> LoggingGuard {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();

    LoggingGuard {}
}
