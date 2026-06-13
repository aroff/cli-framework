use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::Command;
use crate::mcp::schema::McpToolDescriptor;
use crate::security::command_risk::{CommandRiskPolicy, CommandRiskTier};
use crate::security::gate::{ExecutionGate, GateError};
use crate::security::risk_enforcer::PrefightError;
use crate::security::RiskEnforcer;
use crate::spec::value::ArgValue;
use std::collections::HashMap;
use std::sync::Arc;

pub enum BridgeInput {
    Json(serde_json::Value),
    Args(HashMap<String, ArgValue>),
}

#[derive(Clone)]
pub enum ConfirmationMode {
    AssumeYes,
    Ailoop(Arc<AiloopClient>),
    InteractiveStdin,
    NonInteractive,
}

/// Whether the bridge is acting on behalf of an interactive user or the MCP protocol.
///
/// - `Interactive`: runs risk preflight + optional confirmation prompt (chat).
/// - `Mcp`: skips preflight and prompts; the `ExecutionGate` is the
///   authorization point for destructive commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeMode {
    Interactive,
    Mcp,
}

/// Why a confirmation or preflight gate was blocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// User was prompted (ailoop / interactive stdin) and declined.
    UserDeclined,
    /// Non-interactive context with no ailoop available; could not prompt.
    NeedsInteractive,
    /// `ALLOW_DESTRUCTIVE_COMMANDS` is not set.
    EnvGated,
}

pub struct BridgeInvocation<'a> {
    pub command: &'a Command,
    pub input: BridgeInput,
    pub confirmation: ConfirmationMode,
    pub mode: BridgeMode,
}

/// The full outcome of a tool invocation: the captured text output plus any
/// structured-content value the command attached via
/// `AppContext::framework_set_structured_content` (CF-7). The MCP dispatch maps
/// `text` to the result `content` and `structured` to `structuredContent`;
/// callers that only need text (chat) use [`CommandAsToolBridge::invoke`].
#[derive(Debug, Clone, Default)]
pub struct BridgeOutput {
    pub text: String,
    pub structured: Option<serde_json::Value>,
}

#[derive(thiserror::Error, Debug)]
pub enum BridgeError {
    #[error("TOOL_NOT_FOUND: {0}")]
    ToolNotFound(String),

    #[error("ARG_VALIDATION_FAILED: {0}")]
    ArgValidation(String),

    #[error("RISK_REQUIRES_CONFIRMATION: command '{0}' is sensitive and requires confirmation")]
    SensitiveRequiresConfirmation(String, BlockReason),

    #[error("DESTRUCTIVE_BLOCKED: command '{0}' is destructive; requires confirmation")]
    DestructiveBlocked(String, BlockReason),

    #[error("GATE_DENIED: {0}")]
    GateDenied(String),

    #[error("GATE_FAILED: {0}")]
    GateFailed(String),

    #[error("COMMAND_EXECUTION_FAILED: {0}")]
    Execution(#[source] anyhow::Error),
}

pub struct CommandAsToolBridge {
    risk_policy: CommandRiskPolicy,
    gate: Option<Arc<dyn ExecutionGate>>,
}

impl CommandAsToolBridge {
    pub fn new(risk_policy: CommandRiskPolicy) -> Self {
        Self {
            risk_policy,
            gate: None,
        }
    }

    pub fn with_gate(mut self, gate: Arc<dyn ExecutionGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    pub fn describe(&self, tool_name: &str, command: &Command) -> McpToolDescriptor {
        crate::mcp::schema::command_to_tool_descriptor(
            tool_name,
            command.summary(),
            Some(&command.spec),
        )
    }

    pub fn parse_args(
        &self,
        _command: &Command,
        input: BridgeInput,
    ) -> Result<HashMap<String, ArgValue>, BridgeError> {
        match input {
            BridgeInput::Args(args) => Ok(args),
            BridgeInput::Json(value) => {
                let map = match value.as_object() {
                    Some(obj) => crate::mcp::json_value_to_typed_map(obj),
                    None => HashMap::new(),
                };
                Ok(map)
            }
        }
    }

    /// Invoke a command and return its captured text output only.
    ///
    /// Backward-compatible entry point for callers (chat) that do not consume
    /// structured content. For the full result including `structuredContent`
    /// (CF-7), use [`Self::invoke_structured`].
    pub async fn invoke(
        &self,
        ctx: &mut dyn AppContext,
        invocation: BridgeInvocation<'_>,
    ) -> Result<String, BridgeError> {
        self.invoke_structured(ctx, invocation)
            .await
            .map(|o| o.text)
    }

    /// Invoke a command and return its full [`BridgeOutput`] (text +
    /// structured content). The MCP dispatch uses this so a command can return
    /// `structuredContent` distinct from `content` (CF-7).
    pub async fn invoke_structured(
        &self,
        ctx: &mut dyn AppContext,
        invocation: BridgeInvocation<'_>,
    ) -> Result<BridgeOutput, BridgeError> {
        match invocation.mode {
            BridgeMode::Interactive => self.invoke_interactive(ctx, invocation).await,
            BridgeMode::Mcp => self.invoke_mcp(ctx, invocation).await,
        }
    }

    async fn invoke_interactive(
        &self,
        ctx: &mut dyn AppContext,
        invocation: BridgeInvocation<'_>,
    ) -> Result<BridgeOutput, BridgeError> {
        let cmd = invocation.command;
        let args = self.parse_args(cmd, invocation.input)?;

        let diagnostics = crate::app::dispatch::validate_typed_args(cmd, &args);
        if let Some(first) = diagnostics.first() {
            return Err(BridgeError::ArgValidation(first.message.clone()));
        }

        let enforcer = RiskEnforcer::new(self.risk_policy.clone());
        let tier = enforcer.classify(&cmd.id, cmd.category());

        let assume_yes = matches!(invocation.confirmation, ConfirmationMode::AssumeYes);
        let ailoop_available = matches!(invocation.confirmation, ConfirmationMode::Ailoop(_));
        match enforcer.enforce_preflight(&cmd.id, cmd.category(), assume_yes, ailoop_available) {
            Ok(()) => {}
            Err(PrefightError::SensitiveNeedsConfirmation) => {
                return Err(BridgeError::SensitiveRequiresConfirmation(
                    cmd.id.to_string(),
                    BlockReason::NeedsInteractive,
                ));
            }
            Err(PrefightError::DestructiveEnvGated) => {
                return Err(BridgeError::DestructiveBlocked(
                    cmd.id.to_string(),
                    BlockReason::EnvGated,
                ));
            }
            Err(PrefightError::DestructiveNeedsInteractive) => {
                return Err(BridgeError::DestructiveBlocked(
                    cmd.id.to_string(),
                    BlockReason::NeedsInteractive,
                ));
            }
        }

        if !assume_yes {
            match tier {
                CommandRiskTier::Safe => {}
                CommandRiskTier::Sensitive => {
                    request_confirmation(&invocation.confirmation, cmd, false).await?;
                }
                CommandRiskTier::Destructive => {
                    request_confirmation(&invocation.confirmation, cmd, true).await?;
                }
            }
        }

        self.invoke_inner(ctx, cmd, args, tier).await
    }

    async fn invoke_mcp(
        &self,
        ctx: &mut dyn AppContext,
        invocation: BridgeInvocation<'_>,
    ) -> Result<BridgeOutput, BridgeError> {
        let cmd = invocation.command;
        let args = self.parse_args(cmd, invocation.input)?;

        let diagnostics = crate::app::dispatch::validate_typed_args(cmd, &args);
        if let Some(first) = diagnostics.first() {
            return Err(BridgeError::ArgValidation(first.message.clone()));
        }

        let enforcer = RiskEnforcer::new(self.risk_policy.clone());
        let tier = enforcer.classify(&cmd.id, cmd.category());

        self.invoke_inner(ctx, cmd, args, tier).await
    }

    async fn invoke_inner(
        &self,
        ctx: &mut dyn AppContext,
        cmd: &Command,
        args: HashMap<String, ArgValue>,
        tier: CommandRiskTier,
    ) -> Result<BridgeOutput, BridgeError> {
        if let Some(ref gate) = self.gate {
            gate.before_execute(cmd, &args, tier)
                .await
                .map_err(|e| match e {
                    GateError::Denied { reason } => BridgeError::GateDenied(reason),
                    GateError::Failed { reason } => BridgeError::GateFailed(reason),
                })?;
        }

        (cmd.execute)(ctx, args)
            .await
            .map_err(BridgeError::Execution)?;

        Ok(BridgeOutput {
            text: ctx.drain_output(),
            structured: ctx.drain_structured_content(),
        })
    }
}

async fn request_confirmation(
    mode: &ConfirmationMode,
    cmd: &Command,
    destructive: bool,
) -> Result<(), BridgeError> {
    let (action, prompt) = if destructive {
        (
            format!("Execute DESTRUCTIVE command '{}'", cmd.id),
            format!("Execute DESTRUCTIVE command '{}'? [y/N] ", cmd.id),
        )
    } else {
        (
            format!("Execute command '{}'", cmd.id),
            format!("Execute command '{}'? [y/N] ", cmd.id),
        )
    };
    match mode {
        ConfirmationMode::AssumeYes => Ok(()),
        ConfirmationMode::Ailoop(client) => {
            let confirmed = client
                .request_confirmation(&action, None)
                .await
                .map_err(BridgeError::Execution)?;
            if confirmed {
                Ok(())
            } else if destructive {
                Err(BridgeError::DestructiveBlocked(
                    cmd.id.to_string(),
                    BlockReason::UserDeclined,
                ))
            } else {
                Err(BridgeError::SensitiveRequiresConfirmation(
                    cmd.id.to_string(),
                    BlockReason::UserDeclined,
                ))
            }
        }
        ConfirmationMode::InteractiveStdin => {
            let confirmed = prompt_confirm_blocking(prompt)
                .await
                .map_err(BridgeError::Execution)?;
            if confirmed {
                Ok(())
            } else if destructive {
                Err(BridgeError::DestructiveBlocked(
                    cmd.id.to_string(),
                    BlockReason::UserDeclined,
                ))
            } else {
                Err(BridgeError::SensitiveRequiresConfirmation(
                    cmd.id.to_string(),
                    BlockReason::UserDeclined,
                ))
            }
        }
        ConfirmationMode::NonInteractive => {
            if destructive {
                Err(BridgeError::DestructiveBlocked(
                    cmd.id.to_string(),
                    BlockReason::NeedsInteractive,
                ))
            } else {
                Err(BridgeError::SensitiveRequiresConfirmation(
                    cmd.id.to_string(),
                    BlockReason::NeedsInteractive,
                ))
            }
        }
    }
}

async fn prompt_confirm_blocking(prompt: String) -> anyhow::Result<bool> {
    use anyhow::Context;
    use std::io::Write;

    tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        let mut stderr = std::io::stderr();
        write!(stderr, "{}", prompt)?;
        stderr.flush()?;

        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .context("failed to read confirmation from stdin")?;
        let s = line.trim().to_ascii_lowercase();
        Ok(matches!(s.as_str(), "y" | "yes"))
    })
    .await
    .context("confirmation prompt task failed")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
    use crate::spec::command_tree::CommandSpec;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct NoopCtx;
    impl AppContext for NoopCtx {}

    fn noop_execute() -> Arc<
        dyn for<'a> Fn(
                &'a mut dyn crate::app::AppContext,
                HashMap<String, ArgValue>,
            ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
            + Send
            + Sync,
    > {
        Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) }))
    }

    fn make_cmd(id: &'static str, spec: CommandSpec) -> Command {
        Command {
            id: Arc::from(id),
            spec: Arc::new(spec),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: noop_execute(),
        }
    }

    #[tokio::test]
    async fn json_parse_and_spec_validation_failure_returns_arg_validation() {
        let spec = CommandSpec {
            summary: "test",
            args: vec![ArgSpec {
                name: "required",
                kind: ArgKind::Option,
                short: None,
                long: Some("required"),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Required,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "required",
                ..Default::default()
            }],
            ..Default::default()
        };
        let cmd = make_cmd("test", spec);

        let bridge = CommandAsToolBridge::new(CommandRiskPolicy::default());
        let mut ctx = NoopCtx;
        let err = bridge
            .invoke(
                &mut ctx,
                BridgeInvocation {
                    command: &cmd,
                    input: BridgeInput::Json(serde_json::json!({})),
                    confirmation: ConfirmationMode::AssumeYes,
                    mode: BridgeMode::Interactive,
                },
            )
            .await
            .unwrap_err();
        match err {
            BridgeError::ArgValidation(msg) => assert!(msg.contains("missing required argument")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sensitive_command_requires_confirmation_when_noninteractive() {
        let mut policy = CommandRiskPolicy::default();
        policy
            .tiers
            .insert("sensitive".to_string(), CommandRiskTier::Sensitive);
        let cmd = make_cmd(
            "sensitive",
            CommandSpec {
                summary: "test",
                ..Default::default()
            },
        );

        let bridge = CommandAsToolBridge::new(policy);
        let mut ctx = NoopCtx;
        let err = bridge
            .invoke(
                &mut ctx,
                BridgeInvocation {
                    command: &cmd,
                    input: BridgeInput::Args(HashMap::new()),
                    confirmation: ConfirmationMode::NonInteractive,
                    mode: BridgeMode::Interactive,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            BridgeError::SensitiveRequiresConfirmation(_, _)
        ));
    }

    #[tokio::test]
    async fn mcp_mode_skips_preflight_and_prompt() {
        let mut policy = CommandRiskPolicy::default();
        policy
            .tiers
            .insert("sensitive".to_string(), CommandRiskTier::Sensitive);

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_exec = Arc::clone(&calls);
        let cmd = Command {
            id: Arc::from("sensitive"),
            spec: Arc::new(CommandSpec {
                summary: "test",
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: Arc::new(move |_ctx, _args| {
                let calls_for_exec = Arc::clone(&calls_for_exec);
                Box::pin(async move {
                    calls_for_exec.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }),
        };

        let bridge = CommandAsToolBridge::new(policy);
        let mut ctx = NoopCtx;
        bridge
            .invoke(
                &mut ctx,
                BridgeInvocation {
                    command: &cmd,
                    input: BridgeInput::Json(serde_json::json!({})),
                    confirmation: ConfirmationMode::NonInteractive,
                    mode: BridgeMode::Mcp,
                },
            )
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn mcp_mode_with_gate_executes_gate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_gate = Arc::clone(&calls);

        struct CountingGate(Arc<AtomicUsize>);
        #[async_trait::async_trait]
        impl ExecutionGate for CountingGate {
            async fn before_execute(
                &self,
                _cmd: &Command,
                _args: &HashMap<String, ArgValue>,
                _tier: CommandRiskTier,
            ) -> Result<(), GateError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        let cmd = make_cmd(
            "any",
            CommandSpec {
                summary: "test",
                ..Default::default()
            },
        );

        let bridge = CommandAsToolBridge::new(CommandRiskPolicy::default())
            .with_gate(Arc::new(CountingGate(calls_for_gate)));
        let mut ctx = NoopCtx;
        bridge
            .invoke(
                &mut ctx,
                BridgeInvocation {
                    command: &cmd,
                    input: BridgeInput::Json(serde_json::json!({})),
                    confirmation: ConfirmationMode::NonInteractive,
                    mode: BridgeMode::Mcp,
                },
            )
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn block_reason_needs_interactive_on_preflight_block() {
        let mut policy = CommandRiskPolicy::default();
        policy
            .tiers
            .insert("sensitive".to_string(), CommandRiskTier::Sensitive);
        let cmd = make_cmd(
            "sensitive",
            CommandSpec {
                summary: "test",
                ..Default::default()
            },
        );

        let bridge = CommandAsToolBridge::new(policy);
        let mut ctx = NoopCtx;
        let err = bridge
            .invoke(
                &mut ctx,
                BridgeInvocation {
                    command: &cmd,
                    input: BridgeInput::Args(HashMap::new()),
                    confirmation: ConfirmationMode::NonInteractive,
                    mode: BridgeMode::Interactive,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            BridgeError::SensitiveRequiresConfirmation(_, BlockReason::NeedsInteractive)
        ));
    }
}
