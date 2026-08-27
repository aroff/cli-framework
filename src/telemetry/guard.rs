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

#[cfg(not(feature = "telemetry"))]
pub struct TelemetryGuard;
#[cfg(not(feature = "telemetry"))]
impl TelemetryGuard {
    pub fn flush(&self) {}
}
