/// RAII guard that owns the tracer provider pipeline.
///
/// Hold it for as long as spans should be exported; on drop it force-flushes and
/// shuts the provider down so buffered spans are not lost. The builders return
/// one from their run/serve entry points — keep it alive for the process/server
/// lifetime.
#[cfg(feature = "telemetry")]
pub struct TelemetryGuard {
    tracer_provider: opentelemetry_sdk::trace::SdkTracerProvider,
}

#[cfg(feature = "telemetry")]
impl TelemetryGuard {
    pub(crate) fn new(tp: opentelemetry_sdk::trace::SdkTracerProvider) -> Self {
        Self {
            tracer_provider: tp,
        }
    }
    /// Force-flush buffered spans to the exporter without shutting down.
    pub fn flush(&self) {
        let _ = self.tracer_provider.force_flush();
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
