// src/telemetry/exporter.rs
//! The export boundary itself: the last thing a span passes through.
//!
//! `redact_span` is an ordinary function over `SpanData`, and
//! `RedactingExporter` is a wrapper that calls it. Keeping the decision out of
//! the trait impl is what makes it testable without a provider, a runtime, or
//! a collector.

use super::policy::TelemetryPolicy;
use super::redact::{probe_of, RedactionRules, PROBE_ATTR_KEY};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use opentelemetry_sdk::Resource;
use std::sync::Arc;
use std::time::Duration;

/// What the boundary decided about one span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanVerdict {
    Keep,
    Drop,
}

/// Decide a span's fate from the probe it declared.
///
/// A span with no probe is dropped. That is deliberate and it is the strict
/// direction: an unlabelled span is a probe someone forgot to declare, and
/// exporting it would mean shipping data that no entry in the published probe
/// catalog describes. A missing label shows up as missing telemetry, which is
/// noticed; the alternative shows up as undocumented data, which is not.
pub fn span_verdict(policy: &TelemetryPolicy, probe: Option<&str>) -> SpanVerdict {
    match probe {
        Some(id) if policy.effective(id) => SpanVerdict::Keep,
        _ => SpanVerdict::Drop,
    }
}

/// Apply the boundary to one span, in place.
pub fn redact_span(policy: &TelemetryPolicy, span: &mut SpanData) -> SpanVerdict {
    // Fate first: a dropped span is not worth redacting, and redacting before
    // deciding would make "dropped" and "kept but emptied" indistinguishable.
    if span_verdict(policy, probe_of(&span.attributes)) == SpanVerdict::Drop {
        return SpanVerdict::Drop;
    }

    let rules = RedactionRules::from_policy(policy);

    // Events carry their own probe, so a switched-off child probe loses its
    // events without taking the parent span with it.
    span.events
        .events
        .retain(|event| span_verdict(policy, probe_of(&event.attributes)) == SpanVerdict::Keep);
    for event in span.events.events.iter_mut() {
        rules.retain_attributes(&mut event.attributes);
        event
            .attributes
            .retain(|kv| kv.key.as_str() != PROBE_ATTR_KEY);
    }

    rules.retain_attributes(&mut span.attributes);
    span.attributes
        .retain(|kv| kv.key.as_str() != PROBE_ATTR_KEY);

    SpanVerdict::Keep
}

/// A `SpanExporter` that applies the boundary before delegating.
#[derive(Debug)]
pub struct RedactingExporter<E> {
    inner: E,
    policy: Arc<TelemetryPolicy>,
}

impl<E> RedactingExporter<E> {
    pub fn new(inner: E, policy: Arc<TelemetryPolicy>) -> Self {
        Self { inner, policy }
    }

    /// The batch that survives the boundary. Separated from the trait impl so
    /// it can be called directly.
    pub fn filter(&self, batch: Vec<SpanData>) -> Vec<SpanData> {
        batch
            .into_iter()
            .filter_map(|mut span| match redact_span(&self.policy, &mut span) {
                SpanVerdict::Keep => Some(span),
                SpanVerdict::Drop => None,
            })
            .collect()
    }
}

impl<E: SpanExporter> SpanExporter for RedactingExporter<E> {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        let filtered = self.filter(batch);
        async move {
            // Do not hand the inner exporter an empty batch: an OTLP request
            // with no spans is a wasted round trip, and every span here may
            // legitimately have been dropped by the boundary.
            if filtered.is_empty() {
                return Ok(());
            }
            self.inner.export(filtered).await
        }
    }

    fn shutdown_with_timeout(&mut self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn shutdown(&mut self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn force_flush(&mut self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource)
    }
}
