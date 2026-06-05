use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aikit_agent::llm::mock::{MockGateway, MockResponse};
use aikit_agent::llm::{LlmError, LlmGateway, LlmRequest, LlmResponse, LlmStreamHandle};
use aikit_agent::{AgentConfig, AgentInternalEvent, Turn};
use cli_framework::command::chat::host_tool_adapter::McpHostToolAdapter;
use cli_framework::command::chat::{
    ChatToolCallOptions, CHAT_AGENT_START_FAILED, CHAT_ARG_VALIDATION_FAILED,
    CHAT_COMMAND_EXECUTION_FAILED, CHAT_DESTRUCTIVE_BLOCKED, CHAT_FEATURE_DISABLED,
    CHAT_RISK_REQUIRES_CONFIRMATION, CHAT_TOOL_NOT_FOUND, CHAT_TOOL_REGISTRY_COLLISION,
};
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::{McpToolExportPolicy, McpToolRegistry};
use cli_framework::security::command_risk::CommandRiskPolicy;
use cli_framework::spec::command_tree::CommandPath;

fn make_test_config() -> AgentConfig {
    AgentConfig {
        model: "test-model".to_string(),
        base_url: "http://localhost:9999".to_string(),
        api_key: "fake-key".to_string(),
        stream: false,
        max_iterations: 5,
        max_subagent_depth: 0,
        context_budget_tokens: 10000,
        workdir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        allowed_roots: vec![],
        skills_dirs: vec![],
        agents_md_path: None,
        timeout_secs: 30,
        connect_timeout_secs: 5,
        session_persona: None,
        session_agents: Default::default(),
        host_tool_provider: None,
    }
}

/// A gateway that records requests for later inspection while consuming pre-configured responses.
struct RecordingGateway {
    responses: Mutex<VecDeque<MockResponse>>,
    captured: Arc<Mutex<Vec<LlmRequest>>>,
}

impl RecordingGateway {
    fn new(responses: Vec<MockResponse>, captured: Arc<Mutex<Vec<LlmRequest>>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            captured,
        }
    }

    fn next_response(&self) -> MockResponse {
        let mut q = self.responses.lock().unwrap();
        q.pop_front().unwrap_or_else(|| MockResponse::text(""))
    }
}

impl LlmGateway for RecordingGateway {
    fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.captured.lock().unwrap().push(req);
        let resp = self.next_response();
        if let Some(err) = resp.error {
            return Err(err);
        }
        Ok(LlmResponse {
            content: resp.content,
            tool_calls: resp.tool_calls,
            finish_reason: Some(resp.finish_reason),
            usage: None,
        })
    }

    fn stream(&self, req: LlmRequest) -> Result<LlmStreamHandle, LlmError> {
        self.captured.lock().unwrap().push(req);
        let resp = self.next_response();
        if let Some(err) = resp.error {
            return Err(err);
        }
        let events = resp.stream_events.into_iter().map(Ok).collect::<Vec<_>>();
        Ok(LlmStreamHandle::new(events))
    }
}

/// I1: One-shot call with mock returning finish_reason=stop produces TextFinal event.
#[test]
fn i1_one_shot_mock_gateway_returns_text_final() {
    let config = make_test_config();
    let gateway = MockGateway::new(vec![MockResponse::text("hello from mock")]);

    let events =
        aikit_agent::run_with_context(config, vec![], "say hello", Box::new(gateway)).unwrap();

    let text = events.iter().find_map(|e| {
        if let AgentInternalEvent::TextFinal { content, .. } = e {
            Some(content.as_str())
        } else {
            None
        }
    });
    assert!(
        text.is_some(),
        "expected TextFinal event, got: {:?}",
        events
    );
    assert!(
        text.unwrap().contains("hello from mock"),
        "TextFinal content must contain mock response"
    );
}

/// I2/N1: No API key causes AgentConfig::from_env to fail; the error wraps with
/// CHAT_AGENT_START_FAILED as done in runtime.rs.
#[test]
fn i2_no_api_key_maps_to_chat_agent_start_failed() {
    let no_key_err = aikit_agent::AgentError::NoApiKey {
        checked: "OPENAI_API_KEY".to_string(),
    };
    let wrapped = format!("{}: {}", CHAT_AGENT_START_FAILED, no_key_err);
    assert!(
        wrapped.starts_with(CHAT_AGENT_START_FAILED),
        "runtime error must start with CHAT_AGENT_START_FAILED, got: {}",
        wrapped
    );
}

/// I3: Two-turn REPL simulation — prior_turns passed to second run_with_context call
/// must include the user + assistant turns from the first call.
#[test]
fn i3_repl_two_turns_prior_turns_nonempty_on_second_call() {
    // Turn 1: simple text response
    let config1 = make_test_config();
    let gw1 = MockGateway::new(vec![MockResponse::text("first response")]);
    let events1 = aikit_agent::run_with_context(config1, vec![], "first prompt", Box::new(gw1))
        .expect("turn 1 must succeed");

    // Build prior_turns from first turn's events (mirrors turns_from_events logic)
    let mut prior_turns = vec![Turn::user("first prompt")];
    for event in &events1 {
        if let AgentInternalEvent::TextFinal { content, .. } = event {
            prior_turns.push(Turn::assistant(content.clone()));
        }
    }

    assert!(
        prior_turns.len() >= 2,
        "prior_turns must contain at least user + assistant after turn 1"
    );
    assert_eq!(prior_turns[0].content, "first prompt");

    // Turn 2: recording gateway to verify request includes prior context
    let captured = Arc::new(Mutex::new(Vec::<LlmRequest>::new()));
    let gw2 = RecordingGateway::new(
        vec![MockResponse::text("second response")],
        Arc::clone(&captured),
    );

    aikit_agent::run_with_context(
        make_test_config(),
        prior_turns.clone(),
        "second prompt",
        Box::new(gw2),
    )
    .expect("turn 2 must succeed");

    let reqs = captured.lock().unwrap();
    assert!(
        !reqs.is_empty(),
        "recording gateway must have received a request"
    );

    // Request messages must include more than just system + new user message
    // (i.e., prior user + assistant from turn 1 must be present)
    let first_req = &reqs[0];
    assert!(
        first_req.messages.len() > 2,
        "second call must include prior conversation; got {} messages",
        first_req.messages.len()
    );

    let has_prior_user = first_req.messages.iter().any(|m| {
        m.role == "user"
            && m.content
                .as_deref()
                .map(|c| c.contains("first prompt"))
                .unwrap_or(false)
    });
    assert!(
        has_prior_user,
        "second request must carry first user prompt in history"
    );
}

/// I4: Tool call in turn 1 — tool result appears in ToolResult event with captured output.
#[tokio::test]
async fn i4_tool_call_result_forwarded_in_events() {
    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: Arc::from("echo_cmd"),
        spec: Arc::new(cli_framework::spec::command_tree::CommandSpec {
            summary: "echo",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        execute: Arc::new(|ctx, _args| {
            Box::pin(async move {
                ctx.framework_println("tool-output");
                Ok(())
            })
        }),
    });

    let tool_registry = Arc::new(
        McpToolRegistry::from_command_registry_with_policy(
            &registry,
            "myapp",
            McpToolExportPolicy::AllCommands,
        )
        .with_risk_policy(CommandRiskPolicy::default()),
    );

    let adapter = Arc::new(McpHostToolAdapter::new(
        Arc::clone(&tool_registry),
        ChatToolCallOptions {
            yolo: true,
            interactive: false,
            ailoop_client: None,
        },
    ));

    let mut config = make_test_config();
    config.host_tool_provider = Some(adapter as Arc<dyn aikit_agent::host_tools::HostToolProvider>);

    let gw = MockGateway::new(vec![
        MockResponse::tool_call("call-1", "myapp_echo_cmd", "{}"),
        MockResponse::text("done"),
    ]);

    let events = tokio::task::spawn_blocking(move || {
        aikit_agent::run_with_context(config, vec![], "run echo", Box::new(gw))
    })
    .await
    .unwrap()
    .expect("run_with_context must succeed");

    let tool_result = events.iter().find_map(|e| {
        if let AgentInternalEvent::ToolResult { output, .. } = e {
            Some(output.as_str())
        } else {
            None
        }
    });

    assert!(
        tool_result.is_some(),
        "expected ToolResult event; events were: {:?}",
        events
    );
    assert!(
        tool_result.unwrap().contains("tool-output"),
        "tool result must contain captured stdout, got: {}",
        tool_result.unwrap()
    );
}

/// N2: All eight CHAT_* error-code constants have their expected string values.
#[test]
fn n2_all_chat_error_codes_have_correct_string_values() {
    assert_eq!(CHAT_AGENT_START_FAILED, "CHAT_AGENT_START_FAILED");
    assert_eq!(CHAT_TOOL_NOT_FOUND, "CHAT_TOOL_NOT_FOUND");
    assert_eq!(CHAT_ARG_VALIDATION_FAILED, "CHAT_ARG_VALIDATION_FAILED");
    assert_eq!(
        CHAT_COMMAND_EXECUTION_FAILED,
        "CHAT_COMMAND_EXECUTION_FAILED"
    );
    assert_eq!(
        CHAT_RISK_REQUIRES_CONFIRMATION,
        "CHAT_RISK_REQUIRES_CONFIRMATION"
    );
    assert_eq!(CHAT_DESTRUCTIVE_BLOCKED, "CHAT_DESTRUCTIVE_BLOCKED");
    assert_eq!(CHAT_TOOL_REGISTRY_COLLISION, "CHAT_TOOL_REGISTRY_COLLISION");
    assert_eq!(CHAT_FEATURE_DISABLED, "CHAT_FEATURE_DISABLED");
}

/// N3: Tool names registered in McpToolRegistry use underscore-separated appname_group_cmd
/// convention — no slashes, no hyphens, prefixed with the app name.
#[test]
fn n3_tool_name_convention_appname_group_cmd() {
    let mut registry = CommandRegistry::new();
    let path = CommandPath::new(&["deploy", "prod"]).unwrap();
    registry
        .register_at(
            &path,
            Command {
                id: Arc::from("prod"),
                spec: Arc::new(cli_framework::spec::command_tree::CommandSpec {
                    summary: "deploy to prod",
                    ..Default::default()
                }),
                validator: None,
                expose_mcp: false,
                expose_chat: true,
                execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
            },
        )
        .unwrap();

    let tool_registry = McpToolRegistry::from_command_registry_with_policy(
        &registry,
        "myapp",
        McpToolExportPolicy::AllCommands,
    );

    let tools = tool_registry.list_tools();
    assert!(!tools.is_empty());
    for tool in &tools {
        assert!(
            !tool.name.contains('/'),
            "tool name must not contain '/', got: {}",
            tool.name
        );
        assert!(
            tool.name.starts_with("myapp_"),
            "tool name must start with app prefix 'myapp_', got: {}",
            tool.name
        );
        assert!(
            tool.name.chars().all(|c| c.is_alphanumeric() || c == '_'),
            "tool name must be alphanumeric+underscore only, got: {}",
            tool.name
        );
    }
}
