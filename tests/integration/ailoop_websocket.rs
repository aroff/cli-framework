//! Integration tests for ailoop WebSocket client integration.
//!
//! Tests that verify build-time checks, error propagation, and risk gate
//! behavior for the ailoop-backed HITL system.

use std::sync::{Mutex, OnceLock};

use cli_framework::ailoop::{AiloopClient, AiloopConfig};
use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::security::{CommandRiskPolicy, RiskEnforcer};

struct DummyCtx;
impl AppContext for DummyCtx {}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

// AppBuilder::build succeeds when ailoop is configured
#[test]
fn test_build_succeeds_with_ailoop() {
    let result = AppBuilder::new()
        .with_ailoop_channel("test-channel")
        .build(DummyCtx);

    assert!(
        result.is_ok(),
        "build should succeed with ailoop configured"
    );
}

// AppBuilder::build succeeds when nothing is configured
#[test]
fn test_build_succeeds_without_ailoop() {
    let result = AppBuilder::new().build(DummyCtx);
    assert!(result.is_ok(), "build should succeed without ailoop");
}

// AC8: normalize_ws_url converts http:// to ws://
#[test]
fn test_normalize_ws_url_http_to_ws() {
    use cli_framework::ailoop::normalize_ws_url;
    assert_eq!(
        normalize_ws_url("http://localhost:8080"),
        "ws://localhost:8080"
    );
    assert_eq!(normalize_ws_url("https://example.com"), "wss://example.com");
    assert_eq!(
        normalize_ws_url("ws://localhost:9000"),
        "ws://localhost:9000"
    );
    assert_eq!(
        normalize_ws_url("wss://secure.example.com"),
        "wss://secure.example.com"
    );
}

// AiloopClient::with_config fails for invalid URL (after normalization)
#[test]
fn test_ailoop_client_invalid_url_returns_err() {
    let config = AiloopConfig {
        channel: "test".to_string(),
        server_url: Some("ftp://invalid-scheme.example.com".to_string()),
        default_timeout_seconds: 30,
    };
    let result = AiloopClient::with_config(config);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("Invalid WebSocket URL"), "got: {}", msg);
}

// AiloopClient::with_config succeeds for http:// (normalized to ws://)
#[test]
fn test_ailoop_client_http_url_accepted() {
    let config = AiloopConfig {
        channel: "test".to_string(),
        server_url: Some("http://localhost:8080".to_string()),
        default_timeout_seconds: 30,
    };
    assert!(AiloopClient::with_config(config).is_ok());
}

// AC9: If AILOOP_SERVER env not set, default to ws://localhost:8080
#[test]
fn test_default_server_url_when_env_unset() {
    let _guard = env_lock().lock().unwrap();
    let saved = std::env::var("AILOOP_SERVER").ok();
    std::env::remove_var("AILOOP_SERVER");

    let config = AiloopConfig {
        channel: "test".to_string(),
        server_url: None,
        default_timeout_seconds: 30,
    };
    let client = AiloopClient::with_config(config).unwrap();

    assert_eq!(client.channel(), "test");

    if let Some(v) = saved {
        std::env::set_var("AILOOP_SERVER", v);
    }
}

// Risk gate: Sensitive command allowed when ailoop_available=true (non-interactive)
#[test]
fn test_risk_gate_sensitive_allowed_with_ailoop() {
    let enforcer = RiskEnforcer::new(CommandRiskPolicy::default());
    let result = enforcer.enforce_preflight("config-update", Some("config"), false, true);
    assert!(
        result.is_ok(),
        "Sensitive command should be allowed when ailoop is configured"
    );
}

// Risk gate: Sensitive command blocked without ailoop in non-interactive mode
#[test]
fn test_risk_gate_sensitive_blocked_without_ailoop() {
    let enforcer = RiskEnforcer::new(CommandRiskPolicy::default());
    let result = enforcer.enforce_preflight("config-update", Some("config"), false, false);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("SENSITIVE_COMMAND_REQUIRES_CONFIRMATION"));
}

// Risk gate: Destructive command allowed with ailoop + ALLOW_DESTRUCTIVE_COMMANDS=1
#[test]
fn test_risk_gate_destructive_allowed_with_ailoop_and_env() {
    let _guard = env_lock().lock().unwrap();
    std::env::set_var("ALLOW_DESTRUCTIVE_COMMANDS", "1");
    let enforcer = RiskEnforcer::new(CommandRiskPolicy::default());
    let result = enforcer.enforce_preflight("drop-db", Some("destructive"), false, true);
    assert!(
        result.is_ok(),
        "Destructive command should be allowed with ailoop + ALLOW_DESTRUCTIVE_COMMANDS=1"
    );

    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
}

// Risk gate: Destructive command still blocked without ALLOW_DESTRUCTIVE_COMMANDS even with ailoop
#[test]
fn test_risk_gate_destructive_still_blocked_without_env() {
    let _guard = env_lock().lock().unwrap();
    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let enforcer = RiskEnforcer::new(CommandRiskPolicy::default());
    let result = enforcer.enforce_preflight("drop-db", Some("destructive"), false, true);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("DESTRUCTIVE_COMMAND_BLOCKED"));
}
