use crate::command::Command;
use crate::security::command_risk::CommandRiskTier;
use crate::spec::value::ArgValue;
use std::collections::HashMap;

/// Pre-execution authorization gate for command invocations.
///
/// Replaces both `BridgeGate` (tool_bridge) and `McpToolGate` (mcp) with a
/// single unified interface. `transport` is `None` for interactive invocations
/// and `Some(kind)` for MCP transport invocations.
#[async_trait::async_trait]
pub trait ExecutionGate: Send + Sync {
    async fn before_execute(
        &self,
        cmd: &Command,
        args: &HashMap<String, ArgValue>,
        tier: CommandRiskTier,
    ) -> Result<(), GateError>;
}

#[derive(Debug, thiserror::Error)]
pub enum GateError {
    #[error("denied: {reason}")]
    Denied { reason: String },
    #[error("gate failure: {reason}")]
    Failed { reason: String },
}
