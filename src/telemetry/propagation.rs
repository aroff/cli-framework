//! W3C Trace Context propagation across process boundaries (spec 017 R23/R24).
//!
//! A span only joins a caller's trace if the caller's `traceparent` header is
//! read and made the span's parent. Without that, every service mints its own
//! root: an EntityAI → cogni → corpus call produces **three unrelated traces**,
//! each individually correct and collectively useless, which is the state this
//! module exists to end.
//!
//! Three pieces have to line up, and all three are load-bearing:
//!
//! 1. A global `TextMapPropagator` — [`install`], called from telemetry init.
//!    Both [`extract_context`] and [`inject_context`] resolve the propagator
//!    from the OTel global registry, and the default global is a **no-op**. Skip
//!    this and injection writes no header while still returning `Ok`.
//! 2. Extraction on the way in — done for you by the `ApiServer`'s request
//!    layer, which makes the caller's span the parent of `http.request`.
//! 3. Injection on the way out — [`inject_context`], or
//!    [`TracedRequestBuilder::with_trace_context`] on a `reqwest` builder. This
//!    is the one an application has to do itself, because the framework does not
//!    own your HTTP client.
//!
//! ```ignore
//! use cli_framework::telemetry::propagation::TracedRequestBuilder as _;
//!
//! // Inside a handler — the current span is the `http.request` span, so the
//! // downstream service continues this trace rather than starting its own.
//! let resp = client.get(url).with_trace_context().send().await?;
//! ```
//!
//! # Sampling
//!
//! The sampler is `ParentBased`, so a sampled inbound request keeps its whole
//! downstream subtree and an unsampled one drops it. That is the intended
//! behaviour and it only starts working once a parent actually arrives — before
//! propagation existed, `ParentBased` had no parent to be based on and every
//! service sampled independently.
//!
//! # What is deliberately not propagated
//!
//! Only `traceparent`/`tracestate`. **Baggage is not enabled.** Baggage
//! propagates arbitrary caller-supplied key/values to every downstream hop and
//! onward into span attributes; on a multi-tenant platform that is a quiet route
//! for one tenant's identifiers to ride into another service's telemetry. If it
//! is ever wanted it should be an explicit, allowlisted decision, not a default
//! inherited from a composite propagator.

use opentelemetry::trace::TraceContextExt as _;
use opentelemetry::Context;
use opentelemetry_http::{HeaderExtractor, HeaderInjector};

/// Install the global W3C trace-context propagator.
///
/// Idempotent, and called from every telemetry init path — the OTel global
/// propagator defaults to a no-op, so without this both directions silently do
/// nothing.
pub(crate) fn install() {
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
}

/// Read a remote parent context out of inbound request headers.
///
/// Returns an empty `Context` when no valid `traceparent` is present, so callers
/// should check [`has_remote_parent`] before parenting a span to it.
pub fn extract_context(headers: &http::HeaderMap) -> Context {
    opentelemetry::global::get_text_map_propagator(|p| p.extract(&HeaderExtractor(headers)))
}

/// Whether an extracted context actually carries a usable remote span.
///
/// Guards against parenting a span to an empty context: a request arriving with
/// no `traceparent` (a browser, a probe, a curl) must start a fresh root trace,
/// not be attached to whatever the extractor happened to return.
pub fn has_remote_parent(cx: &Context) -> bool {
    cx.span().span_context().is_valid()
}

/// Make `span` continue the caller's trace, if the caller sent one.
///
/// Returns whether a remote parent was attached. This is what the `ApiServer`
/// request layer calls; it is public so a consumer serving another transport
/// (gRPC, a queue consumer) can join traces the same way.
///
/// Must be called before the span is entered. `set_parent` mutates the pending
/// span data the bridge later exports, so a parent attached after the span has
/// started is rejected — see the `AlreadyStarted` arm below.
pub fn continue_trace_from(span: &tracing::Span, headers: &http::HeaderMap) -> bool {
    use tracing_opentelemetry::{OpenTelemetrySpanExt as _, SetParentError};

    let parent = extract_context(headers);
    if !has_remote_parent(&parent) {
        return false;
    }
    match span.set_parent(parent) {
        Ok(()) => true,
        // The subscriber carries no OTel bridge layer. Legitimate and common —
        // the app configured no endpoint, so telemetry is simply off. Init
        // already warns about the one case where this is a mistake (an app that
        // stole the subscriber), so staying quiet here avoids one stderr line
        // per process for every correctly-untraced deployment.
        Err(SetParentError::LayerNotFound) => false,
        // Always a bug: the span was entered before the parent was attached, so
        // this request silently starts a new trace instead of continuing the
        // caller's. Invisible in the logs and indistinguishable downstream from
        // a client that never sent a traceparent, so say it once.
        Err(err) => {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                eprintln!(
                    "cli-framework telemetry: could not attach the caller's trace context \
                     ({err:?}); incoming requests will start new traces instead of continuing \
                     the caller's. The parent must be set before the span is entered."
                );
            });
            false
        }
    }
}

/// Write the **current** span's trace context into outbound request headers.
///
/// No-op when nothing is being traced (telemetry off, or no active span), so it
/// is safe to call unconditionally on any outbound request.
pub fn inject_context(headers: &mut http::HeaderMap) {
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;
    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|p| {
        p.inject_context(&cx, &mut HeaderInjector(headers))
    });
}

/// Attach the current trace context to an outbound `reqwest` request.
///
/// Named `TracedRequestBuilder` rather than `TraceContextExt` to avoid colliding
/// with [`opentelemetry::trace::TraceContextExt`], which is a different trait
/// over `Context` and is frequently in scope alongside this one.
pub trait TracedRequestBuilder {
    /// Add `traceparent` (and `tracestate`, when present) to this request.
    fn with_trace_context(self) -> Self;
}

impl TracedRequestBuilder for reqwest::RequestBuilder {
    fn with_trace_context(self) -> Self {
        let mut headers = http::HeaderMap::new();
        inject_context(&mut headers);
        if headers.is_empty() {
            return self;
        }
        // `headers()` merges rather than replaces, so anything the caller
        // already set survives.
        self.headers(headers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extraction is a pure function of the headers, so it can be checked
    /// without a provider: a well-formed `traceparent` must round-trip into a
    /// context carrying exactly that trace id.
    #[test]
    fn extracts_a_well_formed_traceparent() {
        install();
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .parse()
                .unwrap(),
        );

        let cx = extract_context(&headers);
        assert!(
            has_remote_parent(&cx),
            "valid traceparent was not extracted"
        );
        assert_eq!(
            cx.span().span_context().trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(
            cx.span().span_context().span_id().to_string(),
            "00f067aa0ba902b7"
        );
    }

    /// The case that decides whether an un-traced caller starts a fresh trace or
    /// gets silently welded onto an empty context.
    #[test]
    fn absent_traceparent_yields_no_remote_parent() {
        install();
        let cx = extract_context(&http::HeaderMap::new());
        assert!(
            !has_remote_parent(&cx),
            "a request with no traceparent must not report a remote parent"
        );
    }

    /// A malformed header must be treated as absent, not as a parent with a
    /// garbage trace id — otherwise one bad client poisons trace lookups.
    #[test]
    fn malformed_traceparent_is_ignored() {
        install();
        let mut headers = http::HeaderMap::new();
        headers.insert("traceparent", "not-a-traceparent".parse().unwrap());
        let cx = extract_context(&headers);
        assert!(!has_remote_parent(&cx));
    }

    /// Injection outside any span must not invent a header. If this regresses,
    /// downstream services receive an all-zero parent and their traces break in
    /// a way that looks like a backend bug rather than a client one.
    #[test]
    fn injection_without_an_active_span_writes_nothing() {
        install();
        let mut headers = http::HeaderMap::new();
        inject_context(&mut headers);
        assert!(
            headers.get("traceparent").is_none(),
            "injected a traceparent with no active span: {headers:?}"
        );
    }
}
