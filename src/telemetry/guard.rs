/// RAII guard that owns the OpenTelemetry provider pipelines.
///
/// Hold it for as long as signals should be exported; on drop it force-flushes
/// and shuts the providers down so buffered spans and metrics are not lost. The
/// builders return one from their run/serve entry points — keep it alive for the
/// process/server lifetime.
#[cfg(feature = "telemetry")]
pub struct TelemetryGuard {
    tracer_provider: opentelemetry_sdk::trace::SdkTracerProvider,
    /// `None` when metrics are disabled by config or the metric exporter failed
    /// to build. Traces still work in that case.
    meter_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    /// Upper bound on how long [`Drop`] may block the exiting process.
    ///
    /// `None` — the default, and what a `Service` deployment uses — waits for
    /// the full provider shutdown, because a server that is terminating has a
    /// SIGTERM grace period to spend and losing the last batch of spans is a
    /// real gap in an operator's trace. An end-user CLI has no such budget: a
    /// collector that has stopped responding must not add seconds to `myapp
    /// --version`, so the end-user path sets [`END_USER_FLUSH_BUDGET`].
    flush_budget: Option<std::time::Duration>,
}

#[cfg(feature = "telemetry")]
impl TelemetryGuard {
    pub(crate) fn new(
        tp: opentelemetry_sdk::trace::SdkTracerProvider,
        mp: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    ) -> Self {
        Self {
            tracer_provider: tp,
            meter_provider: mp,
            flush_budget: None,
        }
    }

    /// Bound how long this guard's [`Drop`] may block, or `None` for unbounded.
    ///
    /// Builder-shaped rather than a constructor parameter on purpose: the two
    /// `new` call sites both want the unbounded default, and only the end-user
    /// startup path opts in.
    pub(crate) fn with_flush_budget(mut self, budget: Option<std::time::Duration>) -> Self {
        self.flush_budget = budget;
        self
    }

    /// The drop-time flush budget this guard was built with.
    ///
    /// Exposed for tests: the budget only manifests as *time not spent* on a
    /// dead collector, which is a timing assertion, and a timing assertion is
    /// the one shape of test that passes when the wiring is missing.
    #[doc(hidden)]
    pub fn flush_budget(&self) -> Option<std::time::Duration> {
        self.flush_budget
    }

    /// Force-flush buffered spans and metrics to the exporters without shutting down.
    pub fn flush(&self) {
        let _ = self.tracer_provider.force_flush();
        if let Some(mp) = &self.meter_provider {
            let _ = mp.force_flush();
        }
    }

    /// Return a tracer scoped to this guard's provider pipeline.
    ///
    /// Prefer this over `opentelemetry::global::tracer()` in tests to avoid
    /// races when multiple tests call `set_tracer_provider` concurrently.
    pub fn tracer(&self, name: &'static str) -> opentelemetry_sdk::trace::SdkTracer {
        use opentelemetry::trace::TracerProvider;
        self.tracer_provider.tracer(name)
    }

    /// Flush both providers, but never wait longer than `budget`.
    ///
    /// A collector that stops responding must not hang the process on exit:
    /// the flush runs on its own thread, and a timed-out flush leaves that
    /// thread to finish (or not) on its own rather than blocking the caller
    /// for it.
    pub fn flush_within(&self, budget: std::time::Duration) -> FlushOutcome {
        let tracer_provider = self.tracer_provider.clone();
        let meter_provider = self.meter_provider.clone();
        flush_within(budget, move || {
            let _ = tracer_provider.force_flush();
            if let Some(mp) = &meter_provider {
                let _ = mp.force_flush();
            }
        })
    }
}

#[cfg(feature = "telemetry")]
impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        let meter_provider = self.meter_provider.clone();
        let tracer_provider = self.tracer_provider.clone();
        let shutdown = move || {
            // Metrics first: the periodic reader may still be holding an
            // interval's worth of points, and shutting the tracer down does not
            // flush it.
            if let Some(mp) = &meter_provider {
                let _ = mp.force_flush();
                let _ = mp.shutdown();
            }
            let _ = tracer_provider.force_flush();
            let _ = tracer_provider.shutdown();
        };
        match self.flush_budget {
            // The shutdown calls are inside the budget, not just the flushes: a
            // collector that accepts the connection and then stops reading
            // blocks `shutdown()` exactly as it blocks `force_flush()`, so
            // bounding only the flush would still hang the process on exit.
            Some(budget) => {
                let _ = flush_within(budget, shutdown);
            }
            None => shutdown(),
        }
    }
}

/// Whether a bounded flush finished on its own or was cut off by its budget.
#[cfg(feature = "telemetry")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushOutcome {
    /// The flush closure returned before the budget elapsed.
    Completed,
    /// The budget elapsed first; the flush may still be running in the
    /// background, unobserved.
    TimedOut,
}

/// How long an end-user CLI may spend flushing telemetry on the way out.
///
/// PRD 025: an end-user deployment gets a bounded budget and the exit code is
/// never changed by a flush that misses it; a `Service` deployment gets an
/// unbounded shutdown instead, since it has a SIGTERM grace period to spend.
#[cfg(feature = "telemetry")]
pub const END_USER_FLUSH_BUDGET: std::time::Duration = std::time::Duration::from_millis(500);

/// Run `flush` on its own thread and wait for it, but never longer than
/// `budget`.
///
/// Exit and shutdown paths must not hang the whole process on a collector
/// that stopped responding, so the wait itself — not just the export call —
/// is bounded. A timed-out flush is abandoned: its thread is detached and may
/// finish (or keep blocking) after this function has already returned.
#[cfg(feature = "telemetry")]
pub fn flush_within(
    budget: std::time::Duration,
    flush: impl FnOnce() + Send + 'static,
) -> FlushOutcome {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        flush();
        let _ = tx.send(());
    });
    match rx.recv_timeout(budget) {
        Ok(()) => FlushOutcome::Completed,
        Err(_) => FlushOutcome::TimedOut,
    }
}

/// Test-only hook exercising [`flush_within`] without a real provider pair.
#[cfg(feature = "telemetry")]
#[doc(hidden)]
pub fn flush_within_for_test(
    budget: std::time::Duration,
    flush: impl FnOnce() + Send + 'static,
) -> FlushOutcome {
    flush_within(budget, flush)
}

#[cfg(not(feature = "telemetry"))]
pub struct TelemetryGuard;
#[cfg(not(feature = "telemetry"))]
impl TelemetryGuard {
    pub fn flush(&self) {}
}
