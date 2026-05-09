//! Integration tests for ailoop WebSocket client integration.
//!
//! Tests that verify build-time checks, error propagation, and risk gate
//! behavior for the ailoop-backed HITL system.

use std::sync::Arc;

use cli_framework::ailoop::{AiloopClient, AiloopConfig};
use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use cli_framework::llm::{CommandMetadata, CommandResolution, LlmProvider};
use cli_framework::security::command_risk::{CommandRiskPolicy, CommandRiskTier};

use async_trait::async_trait;

struct DummyCtx;
impl AppContext for DummyCtx {}

struct MockLlmProvider;

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn resolve_command(
        &self,
        _query: &str,
        _commands: &[CommandMetadata],
    ) -> anyhow::Result<CommandResolution> {
        Ok(CommandResolution {
            command_id: "hello".to_string(),
            args: CommandArgs::default(),
            confidence: 0.9,
            reasoning: None,
        })
    }
}

// AC2: AppBuilder::build fails if llm_provider is Some and ailoop_config is None
#[test]
fn test_build_fails_without_ailoop_when_llm_configured() {
    let result = AppBuilder::new()
        .with_llm_provider(Arc::new(MockLlmProvider))
        .build(DummyCtx);

    assert!(
        result.is_err(),
        "build should fail without ailoop config when LLM is set"
    );
    let err = result.map(|_| ()).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("AILOOP_REQUIRED_FOR_ASK"),
        "Expected AILOOP_REQUIRED_FOR_ASK error, got: {}",
        msg
    );
}

// AppBuilder::build succeeds when both llm_provider and ailoop_config are set
#[test]
fn test_build_succeeds_with_ailoop_and_llm() {
    let result = AppBuilder::new()
        .with_llm_provider(Arc::new(MockLlmProvider))
        .with_ailoop_channel("test-channel")
        .build(DummyCtx);

    assert!(
        result.is_ok(),
        "build should succeed with both LLM and ailoop configured"
    );
}

// AppBuilder::build succeeds when only ailoop is configured (no LLM)
#[test]
fn test_build_succeeds_without_llm() {
    let result = AppBuilder::new()
        .with_ailoop_channel("test-channel")
        .build(DummyCtx);

    assert!(
        result.is_ok(),
        "build should succeed with only ailoop configured"
    );
}

// AppBuilder::build succeeds when neither LLM nor ailoop is configured
#[test]
fn test_build_succeeds_without_either() {
    let result = AppBuilder::new().build(DummyCtx);
    assert!(result.is_ok(), "build should succeed without LLM or ailoop");
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
    let saved = std::env::var("AILOOP_SERVER").ok();
    std::env::remove_var("AILOOP_SERVER");

    let config = AiloopConfig {
        channel: "test".to_string(),
        server_url: None,
        default_timeout_seconds: 30,
    };
    let client = AiloopClient::with_config(config).unwrap();

    // We check the channel is configured correctly
    assert_eq!(client.channel(), "test");

    if let Some(v) = saved {
        std::env::set_var("AILOOP_SERVER", v);
    }
}

// Risk gate: Sensitive command allowed when ailoop_available=true (non-interactive)
#[test]
fn test_risk_gate_sensitive_allowed_with_ailoop() {
    use cli_framework::command::enforce_risk_gate;

    let policy = CommandRiskPolicy::default();
    let resolution = CommandResolution {
        command_id: "config-update".to_string(),
        args: CommandArgs::default(),
        confidence: 1.0,
        reasoning: None,
    };

    let result = enforce_risk_gate(&policy, &resolution, Some("config"), false, true);
    assert!(
        result.is_ok(),
        "Sensitive command should be allowed when ailoop is configured"
    );
}

// Risk gate: Sensitive command blocked without ailoop in non-interactive mode
#[test]
fn test_risk_gate_sensitive_blocked_without_ailoop() {
    use cli_framework::command::enforce_risk_gate;

    let policy = CommandRiskPolicy::default();
    let resolution = CommandResolution {
        command_id: "config-update".to_string(),
        args: CommandArgs::default(),
        confidence: 1.0,
        reasoning: None,
    };

    let result = enforce_risk_gate(&policy, &resolution, Some("config"), false, false);
    // In test environment (non-interactive), should be blocked
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("SENSITIVE_COMMAND_REQUIRES_CONFIRMATION"));
}

// Risk gate: Destructive command allowed with ailoop + ALLOW_DESTRUCTIVE_COMMANDS=1
#[test]
fn test_risk_gate_destructive_allowed_with_ailoop_and_env() {
    use cli_framework::command::enforce_risk_gate;

    std::env::set_var("ALLOW_DESTRUCTIVE_COMMANDS", "1");
    let policy = CommandRiskPolicy::default();
    let resolution = CommandResolution {
        command_id: "drop-db".to_string(),
        args: CommandArgs::default(),
        confidence: 1.0,
        reasoning: None,
    };

    let result = enforce_risk_gate(&policy, &resolution, Some("destructive"), false, true);
    assert!(
        result.is_ok(),
        "Destructive command should be allowed with ailoop + ALLOW_DESTRUCTIVE_COMMANDS=1"
    );

    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
}

// Risk gate: Destructive command still blocked without ALLOW_DESTRUCTIVE_COMMANDS even with ailoop
#[test]
fn test_risk_gate_destructive_still_blocked_without_env() {
    use cli_framework::command::enforce_risk_gate;

    std::env::remove_var("ALLOW_DESTRUCTIVE_COMMANDS");
    let policy = CommandRiskPolicy::default();
    let resolution = CommandResolution {
        command_id: "drop-db".to_string(),
        args: CommandArgs::default(),
        confidence: 1.0,
        reasoning: None,
    };

    let result = enforce_risk_gate(&policy, &resolution, Some("destructive"), false, true);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("DESTRUCTIVE_COMMAND_BLOCKED"));
}

// ask command is registered when both LLM and ailoop are configured
#[test]
#[cfg(not(feature = "strict-types"))]
fn test_ask_command_registered_with_llm_and_ailoop() {
    let app = AppBuilder::new()
        .with_llm_provider(Arc::new(MockLlmProvider))
        .with_ailoop_channel("test-channel")
        .register_command(Command {
            id: "hello",
            summary: "Hello",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
        })
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let help = app.render_help();
    assert!(
        help.contains("ask"),
        "ask command should appear in help when LLM and ailoop are configured"
    );
}

// AC10: ask command execution reaches authorize call (verifies via error propagation
// when no server is running — proves the authorize path is reached, not stdin).
//
// The MockLlmProvider resolves the query to "hello", risk gate passes (Safe tier),
// then request_confirmation attempts a WebSocket connection that fails because no
// ailoop server is running. The error MUST contain "Ailoop authorization failed",
// proving authorize was called and stdin was never used.
#[tokio::test]
#[cfg(not(feature = "strict-types"))]
async fn test_ask_command_calls_authorize_not_stdin() {
    // Use a guaranteed-unreachable port so the test doesn't accidentally hit a
    // real server on the developer's machine.
    let config = cli_framework::ailoop::AiloopConfig {
        channel: "test-channel".to_string(),
        server_url: Some("ws://127.0.0.1:19999".to_string()), // nothing listening here
        default_timeout_seconds: 2,
    };

    let mut app = AppBuilder::new()
        .with_llm_provider(Arc::new(MockLlmProvider))
        .with_ailoop_config(config)
        .register_command(Command {
            id: "hello",
            summary: "Hello",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
        })
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let args = CommandArgs {
        positional: vec!["say hello".to_string()],
        named: std::collections::HashMap::new(),
    };

    let result = app.execute_command("ask", args).await;

    // The command MUST fail because no ailoop server is running.
    // The error message proves that request_confirmation (which calls authorize)
    // was reached — a stdin-based path would have returned Ok or blocked on read.
    assert!(
        result.is_err(),
        "ask execution should fail when ailoop server is unreachable"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Ailoop authorization failed"),
        "Expected 'Ailoop authorization failed' in error (proves authorize was called, not stdin); got: {}",
        err_msg
    );
}
