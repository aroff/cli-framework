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

/// Returned by [`LoggingGuard::attach_otel_layer`] when the slot already holds
/// a layer.
///
/// The slot is write-once by design (see [`attach_once`]), so a second attach
/// is a programming error rather than something to degrade around: the layer
/// already in place keeps working and the caller is told, instead of the
/// second layer being silently dropped or the first silently replaced.
#[cfg(feature = "telemetry")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyAttached;

#[cfg(feature = "telemetry")]
impl std::fmt::Display for AlreadyAttached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an OpenTelemetry layer is already attached to this process's subscriber")
    }
}

#[cfg(feature = "telemetry")]
impl std::error::Error for AlreadyAttached {}

#[cfg(feature = "telemetry")]
mod attach_once {
    //! A write-once slot for the OpenTelemetry bridge layer.
    //!
    //! `tracing_subscriber::reload::Layer` is the obvious thing to reach for
    //! here, and it is the wrong tool. Its `downcast_raw` deliberately answers
    //! `None` for every type but `NoneLayerMarker`, because a reloadable slot
    //! can be replaced at any moment and a pointer handed out through it could
    //! dangle. `tracing-opentelemetry` reaches its layer *only* through that
    //! downcast: the layer registers a `WithContext`, and
    //! `OpenTelemetrySpanExt::context`, `set_parent`, `set_status`,
    //! `add_link` and `set_attribute` each look it up and return silently when
    //! it is missing.
    //!
    //! Bridging through a reload slot therefore produces a subscriber that
    //! still exports spans but carries no trace context:
    //! `set_parent_from_headers` cannot join an inbound trace and
    //! `inject_context` writes no `traceparent`, both without an error, and
    //! neither is visible until someone opens a distributed trace and finds
    //! three unrelated ones — precisely the failure
    //! [`crate::telemetry::propagation`] exists to end.
    //!
    //! Writing the slot once and never replacing it is what makes forwarding
    //! the downcast sound: the boxed layer is owned by an `Arc` that the live
    //! subscriber holds for as long as it exists, so a pointer into it cannot
    //! be invalidated while anything can still use it.

    use std::any::TypeId;
    use std::sync::{Arc, OnceLock};

    use tracing::span;
    use tracing::Event;
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::{Layer, Registry};

    use super::{AlreadyAttached, BoxedLayer};

    #[derive(Clone)]
    pub(super) struct AttachOnceSlot(Arc<OnceLock<BoxedLayer>>);

    impl AttachOnceSlot {
        pub(super) fn new() -> Self {
            Self(Arc::new(OnceLock::new()))
        }

        /// Fill the slot. Fails if it is already full; never replaces.
        pub(super) fn attach(&self, layer: BoxedLayer) -> Result<(), AlreadyAttached> {
            self.0.set(layer).map_err(|_| AlreadyAttached)
        }
    }

    /// The slot is deliberately transparent to filtering: it overrides neither
    /// `register_callsite`, `enabled` nor `max_level_hint`. Those answers are
    /// cached process-wide the first time each callsite is seen, which is
    /// before the layer is attached; a slot that changed its mind at attach
    /// time would leave those caches describing a subscriber that no longer
    /// exists. Filtering stays the `EnvFilter`'s job, at every point in time.
    impl Layer<Registry> for AttachOnceSlot {
        fn on_new_span(
            &self,
            attrs: &span::Attributes<'_>,
            id: &span::Id,
            ctx: Context<'_, Registry>,
        ) {
            if let Some(inner) = self.0.get() {
                inner.on_new_span(attrs, id, ctx);
            }
        }

        fn on_record(&self, id: &span::Id, values: &span::Record<'_>, ctx: Context<'_, Registry>) {
            if let Some(inner) = self.0.get() {
                inner.on_record(id, values, ctx);
            }
        }

        fn on_follows_from(&self, id: &span::Id, follows: &span::Id, ctx: Context<'_, Registry>) {
            if let Some(inner) = self.0.get() {
                inner.on_follows_from(id, follows, ctx);
            }
        }

        fn event_enabled(&self, event: &Event<'_>, ctx: Context<'_, Registry>) -> bool {
            match self.0.get() {
                Some(inner) => inner.event_enabled(event, ctx),
                None => true,
            }
        }

        fn on_event(&self, event: &Event<'_>, ctx: Context<'_, Registry>) {
            if let Some(inner) = self.0.get() {
                inner.on_event(event, ctx);
            }
        }

        fn on_enter(&self, id: &span::Id, ctx: Context<'_, Registry>) {
            if let Some(inner) = self.0.get() {
                inner.on_enter(id, ctx);
            }
        }

        fn on_exit(&self, id: &span::Id, ctx: Context<'_, Registry>) {
            if let Some(inner) = self.0.get() {
                inner.on_exit(id, ctx);
            }
        }

        fn on_close(&self, id: span::Id, ctx: Context<'_, Registry>) {
            if let Some(inner) = self.0.get() {
                inner.on_close(id, ctx);
            }
        }

        fn on_id_change(&self, old: &span::Id, new: &span::Id, ctx: Context<'_, Registry>) {
            if let Some(inner) = self.0.get() {
                inner.on_id_change(old, new, ctx);
            }
        }

        unsafe fn downcast_raw(&self, id: TypeId) -> Option<*const ()> {
            if id == TypeId::of::<Self>() {
                return Some(std::ptr::from_ref(self).cast());
            }
            // SAFETY: forwarding is what lets `tracing-opentelemetry` find its
            // `WithContext`, and it is sound here because the pointer cannot be
            // invalidated while the borrow that produced it is usable: the slot
            // is written at most once, the `Arc` keeps the box alive for as
            // long as the subscriber holding this layer, and nothing ever
            // replaces or drops the boxed layer while that subscriber exists.
            unsafe { self.0.get()?.downcast_raw(id) }
        }
    }
}

/// Process-global copy of the slot [`install_default_logging`] layered in.
///
/// Telemetry startup runs inside `App::run_with_args`, which never sees the
/// [`LoggingGuard`] the application's `main` is holding. Without a global copy
/// the upgrade path would be reachable only by applications that thread that
/// guard down into the framework — exactly the boilerplate
/// `init_default_logging` exists to remove.
#[cfg(feature = "telemetry")]
static GLOBAL_ATTACH_SLOT: std::sync::OnceLock<attach_once::AttachOnceSlot> =
    std::sync::OnceLock::new();

/// Attach `layer` to the subscriber [`install_default_logging`] installed
/// earlier in this process.
///
/// Returns `false` when there is no slot to attach to — either
/// `init_default_logging()` was never called, or it lost the process global to
/// a foreign subscriber — and also when a layer is already attached. Any
/// `false` here is a real loss of trace export, so the caller reports
/// [`SubscriberOutcome::ForeignSubscriber`] rather than pretending the layer
/// landed.
#[cfg(feature = "telemetry")]
pub fn attach_otel_layer_globally(layer: BoxedLayer) -> bool {
    match GLOBAL_ATTACH_SLOT.get() {
        Some(slot) => slot.attach(layer).is_ok(),
        None => false,
    }
}

/// Returned by [`crate::init_default_logging`].
///
/// It carries the write-once slot that telemetry startup fills with the OTel
/// layer, which is how an application can call `init_default_logging()` in
/// `main` and still get exported traces. Holding it is not required for
/// logging to work; dropping it only gives up that upgrade path.
#[must_use = "hold the guard to let telemetry attach its layer later"]
pub struct LoggingGuard {
    #[cfg(feature = "telemetry")]
    slot: Option<attach_once::AttachOnceSlot>,
}

impl LoggingGuard {
    /// Whether telemetry startup can still attach the OTel layer to this
    /// process's subscriber. `false` when [`install_default_logging`] lost
    /// the process global to something else, in which case there is no slot
    /// left to attach to (the foreign subscriber owns the composition).
    ///
    /// Under the weaker `observability`-only build there is no slot at all
    /// — `init_default_logging` cannot be upgraded by telemetry that was
    /// never compiled in — so this is always `false`.
    pub fn can_attach_otel_layer(&self) -> bool {
        #[cfg(feature = "telemetry")]
        {
            self.slot.is_some()
        }
        #[cfg(not(feature = "telemetry"))]
        {
            false
        }
    }

    /// Attach `layer` to this process's subscriber.
    ///
    /// A no-op, not an error, when
    /// [`can_attach_otel_layer`](Self::can_attach_otel_layer) is `false` —
    /// there is no slot to attach to (a foreign subscriber won the install),
    /// and this module's rule throughout is to degrade rather than fail when
    /// that happens.
    ///
    /// Errors only when a layer is already attached: the slot is write-once,
    /// because a replaceable one cannot forward the downcast
    /// `tracing-opentelemetry` needs (see [`attach_once`]).
    #[cfg(feature = "telemetry")]
    pub fn attach_otel_layer(&self, layer: BoxedLayer) -> Result<(), AlreadyAttached> {
        match &self.slot {
            Some(slot) => slot.attach(layer),
            None => Ok(()),
        }
    }
}

/// The new body of `crate::init_default_logging`: install a process-wide
/// subscriber and hand back a guard.
///
/// Rule 1: an application that never asks for telemetry gets what it has
/// today. Under `telemetry`, the subscriber additionally carries a write-once
/// slot so telemetry startup can attach the OTel layer later without a second,
/// competing `try_init` call.
#[cfg(feature = "telemetry")]
pub fn install_default_logging() -> LoggingGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let slot = attach_once::AttachOnceSlot::new();

    // The slot must be the first layer added on top of the bare `Registry`:
    // it implements `Layer<Registry>` for exactly that subscriber, not for
    // whatever stack happens to be built by the time `.with()` reaches it.
    let installed = tracing_subscriber::registry()
        .with(slot.clone())
        .with(compose::filter())
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .try_init()
        .is_ok();

    if installed {
        // Publish before returning: `App::run_with_args` reaches the slot
        // through the global, not through the guard the caller keeps.
        let _ = GLOBAL_ATTACH_SLOT.set(slot.clone());
    }

    LoggingGuard {
        slot: if installed { Some(slot) } else { None },
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
