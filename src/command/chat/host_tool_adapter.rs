use crate::command::chat::ChatToolCallOptions;
use crate::mcp::McpToolRegistry;
use aikit_agent::host_tools::{HostToolDefinition, HostToolProvider};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// AppContext implementation that captures output from `framework_println` calls.
struct CaptureCtx {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl CaptureCtx {
    fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl crate::app::context::AppContext for CaptureCtx {
    fn framework_println(&self, s: &str) {
        use std::io::Write;
        let mut buf = self.buffer.lock().unwrap();
        let _ = writeln!(buf, "{}", s);
    }

    fn drain_output(&self) -> String {
        let mut buf = self.buffer.lock().unwrap();
        let data = std::mem::take(&mut *buf);
        String::from_utf8_lossy(&data).into_owned()
    }
}

/// Adapts `McpToolRegistry` to implement aikit-agent's `HostToolProvider` trait.
///
/// This bridges the cli-framework command execution pipeline into the aikit-agent
/// loop, forwarding tool output captured via `AppContext::framework_println`.
pub struct McpHostToolAdapter {
    registry: Arc<McpToolRegistry>,
    opts: ChatToolCallOptions,
}

impl McpHostToolAdapter {
    pub fn new(registry: Arc<McpToolRegistry>, opts: ChatToolCallOptions) -> Self {
        Self { registry, opts }
    }
}

impl McpHostToolAdapter {
    /// Inherent wrapper so callers don't need `HostToolProvider` in scope.
    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
        <Self as HostToolProvider>::call_tool(self, name, arguments)
    }

    /// Inherent wrapper so callers don't need `HostToolProvider` in scope.
    pub fn list_tools(&self) -> Vec<HostToolDefinition> {
        <Self as HostToolProvider>::list_tools(self)
    }
}

impl HostToolProvider for McpHostToolAdapter {
    fn list_tools(&self) -> Vec<HostToolDefinition> {
        self.registry
            .list_tools()
            .into_iter()
            .map(|t| HostToolDefinition {
                name: t.name,
                description: Some(t.description),
                parameters: t.input_schema,
            })
            .collect()
    }

    fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
        use crate::command::chat::{
            CHAT_ARG_VALIDATION_FAILED, CHAT_COMMAND_EXECUTION_FAILED, CHAT_DESTRUCTIVE_BLOCKED,
            CHAT_RISK_REQUIRES_CONFIRMATION, CHAT_TOOL_NOT_FOUND,
        };
        use crate::command_surface::tool_bridge::{
            BlockReason, BridgeError, BridgeInput, BridgeInvocation, BridgeMode,
            CommandAsToolBridge, ConfirmationMode,
        };

        let cmd = self
            .registry
            .resolve_tool(name)
            .ok_or_else(|| format!("{}: tool '{}' not registered", CHAT_TOOL_NOT_FOUND, name))?;

        let confirmation = if self.opts.yolo {
            ConfirmationMode::AssumeYes
        } else if let Some(ref client) = self.opts.ailoop_client {
            ConfirmationMode::Ailoop(Arc::clone(client))
        } else if self.opts.interactive {
            ConfirmationMode::InteractiveStdin
        } else {
            ConfirmationMode::NonInteractive
        };

        let bridge = CommandAsToolBridge::new(self.registry.risk_policy().clone());
        let mut capture_ctx = CaptureCtx::new();

        // run_with_context (and our tests via spawn_blocking) call this from a
        // blocking-thread context, so Handle::current().block_on() is valid here.
        let handle = tokio::runtime::Handle::current();
        let result = handle.block_on(bridge.invoke(
            &mut capture_ctx,
            BridgeInvocation {
                command: cmd,
                input: BridgeInput::Json(arguments),
                confirmation,
                mode: BridgeMode::Interactive,
            },
        ));

        match result {
            Ok(output) => Ok(output),
            Err(BridgeError::ArgValidation(msg)) => {
                Err(format!("{}: {}", CHAT_ARG_VALIDATION_FAILED, msg))
            }
            Err(BridgeError::SensitiveRequiresConfirmation(cmd_id, reason)) => match reason {
                BlockReason::UserDeclined => Err(format!(
                    "{}: user declined confirmation for '{}'",
                    CHAT_RISK_REQUIRES_CONFIRMATION, cmd_id
                )),
                _ => Err(format!(
                    "{}: command '{}' is sensitive and requires confirmation",
                    CHAT_RISK_REQUIRES_CONFIRMATION, cmd_id
                )),
            },
            Err(BridgeError::DestructiveBlocked(cmd_id, reason)) => match reason {
                BlockReason::UserDeclined => Err(format!(
                    "{}: user declined confirmation for '{}'",
                    CHAT_DESTRUCTIVE_BLOCKED, cmd_id
                )),
                _ => Err(format!(
                    "{}: command '{}' is destructive; gated by ALLOW_DESTRUCTIVE_COMMANDS",
                    CHAT_DESTRUCTIVE_BLOCKED, cmd_id
                )),
            },
            Err(BridgeError::Execution(e)) => {
                Err(format!("{}: {}", CHAT_COMMAND_EXECUTION_FAILED, e))
            }
            Err(other) => Err(format!("{}: {}", CHAT_COMMAND_EXECUTION_FAILED, other)),
        }
    }
}
