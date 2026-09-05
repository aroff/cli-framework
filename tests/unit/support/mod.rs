// tests/unit/support/mod.rs
//! Shared policy fixtures for the export-boundary tests.
//!
//! `TelemetryPolicy` has many fields and almost every test cares about two of
//! them. This builds a valid policy and lets the caller mutate the fields that
//! matter, so a new field on the struct does not touch thirty tests.

use cli_framework::telemetry::{
    Attribution, Deployment, ProbeRegistry, TelemetryInputs, TelemetryLevel, TelemetryPolicy,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

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
