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
        }
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
        // Metrics first: the periodic reader may still be holding an interval's
        // worth of points, and shutting the tracer down does not flush it.
        if let Some(mp) = &self.meter_provider {
            let _ = mp.force_flush();
            let _ = mp.shutdown();
        }
        let _ = self.tracer_provider.force_flush();
        let _ = self.tracer_provider.shutdown();
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
