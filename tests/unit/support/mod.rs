// tests/unit/support/mod.rs
//! Shared policy fixtures for the export-boundary tests.
//!
//! `TelemetryPolicy` has many fields and almost every test cares about two of
//! them. This builds a valid policy and lets the caller mutate the fields that
//! matter, so a new field on the struct does not touch thirty tests.

use cli_framework::telemetry::{
    Attribution, Deployment, ProbeRegistry, TelemetryInputs, TelemetryLevel, TelemetryPolicy,
};
use opentelemetry::trace::{SpanContext, SpanKind, Status};
use opentelemetry::KeyValue;
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};
use std::borrow::Cow;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::SystemTime;

pub fn policy_with(
    deployment: Deployment,
    level: TelemetryLevel,
    tweak: impl FnOnce(&mut TelemetryPolicy),
) -> TelemetryPolicy {
    let mut policy = cli_framework::telemetry::resolve_policy(TelemetryInputs {
        app: "demo".to_string(),
        deployment,
        endpoint: Some("http://collector:4318".to_string()),
        attribution: Attribution::Pseudonymous,
        install_id: Some("install-fixture".to_string()),
        session_id: "session-fixture".to_string(),
        registry: ProbeRegistry::with_builtins(),
        store_available: true,
        ..Default::default()
    });
    // Set directly rather than by layering: these tests are about the
    // boundary's behaviour at a telemetry level, not about how that level was
    // reached — PR1's resolver tests own that.
    policy.level = level;
    tweak(&mut policy);
    policy
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// RAII helper for tests that must observe a real process environment
/// variable.
///
/// `resource.rs`'s `metric_resource_attrs` reads `OTEL_SERVICE_NAME` straight
/// from `std::env`, matching how every other OTel SDK resolves it, so there
/// is no injected-closure seam to use instead here. The environment is
/// global, mutable state shared by every test in the binary, so construction
/// takes a process-wide lock held for the guard's whole lifetime —
/// serializing any two tests that touch the environment this way, not just
/// two tests touching the same key — and `Drop` restores exactly what was
/// there before (absent or present), never assuming unset.
pub struct EnvGuard {
    key: String,
    prior: Option<String>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    /// Set `key` to `value` for the lifetime of the returned guard.
    pub fn set(key: &str, value: &str) -> Self {
        let lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var(key).ok();
        std::env::set_var(key, value);
        EnvGuard {
            key: key.to_string(),
            prior,
            _lock: lock,
        }
    }

    /// Remove `key` for the lifetime of the returned guard.
    pub fn unset(key: &str) -> Self {
        let lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var(key).ok();
        std::env::remove_var(key);
        EnvGuard {
            key: key.to_string(),
            prior,
            _lock: lock,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prior {
            Some(v) => std::env::set_var(&self.key, v),
            None => std::env::remove_var(&self.key),
        }
    }
}

pub fn attrs(pairs: &[(&str, &str)]) -> Vec<KeyValue> {
    pairs
        .iter()
        .map(|(k, v)| KeyValue::new(k.to_string(), v.to_string()))
        .collect()
}

/// A minimal `SpanData` — enough for the boundary, which only reads the name,
/// the attributes and the events.
pub fn span_named(name: &str, pairs: &[(&str, &str)]) -> SpanData {
    SpanData {
        span_context: SpanContext::empty_context(),
        parent_span_id: opentelemetry::trace::SpanId::INVALID,
        // Not in the plan's snippet: opentelemetry_sdk 0.31's `SpanData` adds
        // this field, and omitting it is a missing-field compile error.
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: Cow::Owned(name.to_string()),
        start_time: SystemTime::UNIX_EPOCH,
        end_time: SystemTime::UNIX_EPOCH,
        attributes: attrs(pairs),
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: opentelemetry::InstrumentationScope::builder("test").build(),
    }
}

pub fn event(name: &str, pairs: &[(&str, &str)]) -> opentelemetry::trace::Event {
    opentelemetry::trace::Event::new(name.to_string(), SystemTime::UNIX_EPOCH, attrs(pairs), 0)
}

/// An in-memory `SpanExporter` that records the names it was handed.
#[derive(Debug)]
pub struct RecordingExporter {
    seen: Arc<Mutex<Vec<String>>>,
}

impl RecordingExporter {
    pub fn new(seen: Arc<Mutex<Vec<String>>>) -> Self {
        Self { seen }
    }
}

impl opentelemetry_sdk::trace::SpanExporter for RecordingExporter {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = opentelemetry_sdk::error::OTelSdkResult> + Send {
        let seen = self.seen.clone();
        async move {
            let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
            seen.extend(batch.into_iter().map(|span| span.name.to_string()));
            Ok(())
        }
    }
}

/// Drive one `export` call to completion without a Tokio runtime.
pub fn export_blocking<E: opentelemetry_sdk::trace::SpanExporter>(
    exporter: &E,
    batch: Vec<SpanData>,
) {
    futures_executor::block_on(exporter.export(batch)).expect("export must not fail in a test");
}
