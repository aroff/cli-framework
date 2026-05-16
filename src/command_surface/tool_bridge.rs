use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs};
use crate::mcp::schema::McpToolDescriptor;
use crate::security::command_risk::{CommandRiskPolicy, CommandRiskTier};
use crate::security::RiskEnforcer;
use crate::spec::value::ArgValue;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BridgeSemantics {
    /// Ask/chat semantics: enforce risk preflight and prompt/confirm for sensitive/destructive.
    AskChat,
    /// MCP semantics: never enforce preflight and never prompt/confirm.
    Mcp,
}

pub enum BridgeInput {
    Json(serde_json::Value),
    Args(CommandArgs),
}

#[derive(Clone)]
pub enum ConfirmationMode {
    AssumeYes,
    Ailoop(Arc<AiloopClient>),
    InteractiveStdin,
    NonInteractive,
}

pub struct ParsedArgs {
    pub args: CommandArgs,
    pub typed: Option<HashMap<String, ArgValue>>,
}

pub struct BridgeInvocation<'a> {
    pub command: &'a Command,
    pub input: BridgeInput,
    pub confirmation: ConfirmationMode,
}

#[derive(thiserror::Error, Debug)]
pub enum BridgeError {
    #[error("TOOL_NOT_FOUND: {0}")]
    ToolNotFound(String),

    #[error("ARG_VALIDATION_FAILED: {0}")]
    ArgValidation(String),

    #[error("RISK_REQUIRES_CONFIRMATION: command '{0}' is sensitive and requires confirmation")]
    SensitiveRequiresConfirmation(String),

    #[error("DESTRUCTIVE_BLOCKED: command '{0}' is destructive; requires confirmation")]
    DestructiveBlocked(String),

    #[error("GATE_DENIED: {0}")]
    GateDenied(String),

    #[error("GATE_FAILED: {0}")]
    GateFailed(String),

    #[error("COMMAND_EXECUTION_FAILED: {0}")]
    Execution(#[source] anyhow::Error),
}

#[async_trait]
pub trait BridgeGate: Send + Sync {
    async fn before_execute(
        &self,
        cmd: &Command,
        args: &CommandArgs,
        tier: CommandRiskTier,
    ) -> Result<(), BridgeError>;
}

pub struct CommandAsToolBridge {
    risk_policy: CommandRiskPolicy,
    gate: Option<Arc<dyn BridgeGate>>,
    semantics: BridgeSemantics,
}

impl CommandAsToolBridge {
    pub fn new(risk_policy: CommandRiskPolicy) -> Self {
        Self {
            risk_policy,
            gate: None,
            semantics: BridgeSemantics::AskChat,
        }
    }

    pub fn with_gate(mut self, gate: Arc<dyn BridgeGate>) -> Self {
        self.gate = Some(gate);
        // Gate is only used by MCP today; treat any bridged gate as MCP semantics.
        // If a future surface wants ask/chat semantics plus a gate, it should call a
        // separate entry point in a follow-up spec/ADR.
        self.semantics = BridgeSemantics::Mcp;
        self
    }

    pub fn describe(&self, tool_name: &str, command: &Command) -> McpToolDescriptor {
        crate::mcp::schema::command_to_tool_descriptor(
            tool_name,
            command.summary,
            command.spec.as_deref(),
        )
    }

    pub fn parse_args(
        &self,
        command: &Command,
        input: BridgeInput,
    ) -> Result<ParsedArgs, BridgeError> {
        let _ = command;
        match input {
            BridgeInput::Args(args) => Ok(ParsedArgs { args, typed: None }),
            BridgeInput::Json(value) => {
                let (args, typed) = crate::mcp::map_mcp_args_to_command_args_from_json(value)
                    .map_err(|e| BridgeError::ArgValidation(e.to_string()))?;
                Ok(ParsedArgs {
                    args,
                    typed: Some(typed),
                })
            }
        }
    }

    pub async fn invoke(
        &self,
        ctx: &mut dyn AppContext,
        invocation: BridgeInvocation<'_>,
    ) -> Result<(), BridgeError> {
        let cmd = invocation.command;

        let parsed = self.parse_args(cmd, invocation.input)?;

        // Typed validation rules vary per surface and MUST remain stable (§4.1).
        if let Some(ref typed) = parsed.typed {
            // MCP: validate only when spec is present.
            // Chat: validate when spec OR custom validator exists.
            let should_validate = if self.semantics == BridgeSemantics::Mcp {
                cmd.spec.is_some()
            } else {
                cmd.spec.is_some() || cmd.validator.is_some()
            };
            if should_validate {
                let diagnostics = crate::app::dispatch::validate_typed_args(cmd, typed)
                    .map_err(|e| BridgeError::ArgValidation(e.to_string()))?;
                if let Some(first) = diagnostics.first() {
                    return Err(BridgeError::ArgValidation(first.message.clone()));
                }
            }
        }

        let enforcer = RiskEnforcer::new(self.risk_policy.clone());
        let tier = enforcer.classify(cmd.id, cmd.category);

        // Ask/chat: enforce preflight + confirmation; MCP: never enforce or prompt (§4.1).
        if self.semantics == BridgeSemantics::AskChat {
            let assume_yes = matches!(invocation.confirmation, ConfirmationMode::AssumeYes);
            let ailoop_available = matches!(
                invocation.confirmation,
                ConfirmationMode::Ailoop(_) | ConfirmationMode::InteractiveStdin
            );
            if let Err(e) =
                enforcer.enforce_preflight(cmd.id, cmd.category, assume_yes, ailoop_available)
            {
                // Preflight currently only emits these two error families.
                let msg = e.to_string();
                if msg.starts_with("SENSITIVE_COMMAND_REQUIRES_CONFIRMATION:") {
                    return Err(BridgeError::SensitiveRequiresConfirmation(
                        cmd.id.to_string(),
                    ));
                }
                return Err(BridgeError::DestructiveBlocked(cmd.id.to_string()));
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
        }

        if let Some(ref gate) = self.gate {
            gate.before_execute(cmd, &parsed.args, tier).await?;
        }

        let effective_args = match parsed.typed.as_ref() {
            Some(typed) => {
                crate::app::dispatch::effective_args_for_execution(parsed.args.clone(), Some(typed))
            }
            None => parsed.args,
        };

        (cmd.execute)(ctx, effective_args)
            .await
            .map_err(BridgeError::Execution)?;
        Ok(())
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
                Err(BridgeError::DestructiveBlocked(cmd.id.to_string()))
            } else {
                Err(BridgeError::SensitiveRequiresConfirmation(
                    cmd.id.to_string(),
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
                Err(BridgeError::DestructiveBlocked(cmd.id.to_string()))
            } else {
                Err(BridgeError::SensitiveRequiresConfirmation(
                    cmd.id.to_string(),
                ))
            }
        }
        ConfirmationMode::NonInteractive => {
            if destructive {
                Err(BridgeError::DestructiveBlocked(cmd.id.to_string()))
            } else {
                Err(BridgeError::SensitiveRequiresConfirmation(
                    cmd.id.to_string(),
                ))
            }
        }
    }
}

async fn prompt_confirm_blocking(prompt: String) -> anyhow::Result<bool> {
    use anyhow::Context;
    use std::io::Write;

    // Blocking stdin read must not run on the async runtime (§4.3).
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
                CommandArgs,
            ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
            + Send
            + Sync,
    > {
        Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) }))
    }

    #[tokio::test]
    async fn json_parse_and_spec_validation_failure_returns_arg_validation() {
        let spec = CommandSpec {
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
            }],
            ..Default::default()
        };
        let cmd = Command {
            id: "test",
            summary: "test",
            syntax: None,
            category: None,
            spec: Some(Arc::new(spec)),
            validator: None,
            expose_mcp: false,
            execute: noop_execute(),
        };

        let bridge = CommandAsToolBridge::new(CommandRiskPolicy::default());
        let mut ctx = NoopCtx;
        let err = bridge
            .invoke(
                &mut ctx,
                BridgeInvocation {
                    command: &cmd,
                    input: BridgeInput::Json(serde_json::json!({})),
                    confirmation: ConfirmationMode::AssumeYes,
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
        let cmd = Command {
            id: "sensitive",
            summary: "test",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: noop_execute(),
        };

        let bridge = CommandAsToolBridge::new(policy);
        let mut ctx = NoopCtx;
        let err = bridge
            .invoke(
                &mut ctx,
                BridgeInvocation {
                    command: &cmd,
                    input: BridgeInput::Args(CommandArgs::default()),
                    confirmation: ConfirmationMode::NonInteractive,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, BridgeError::SensitiveRequiresConfirmation(_)));
    }

    #[tokio::test]
    async fn mcp_calls_never_enforce_preflight_or_prompt() {
        let mut policy = CommandRiskPolicy::default();
        policy
            .tiers
            .insert("sensitive".to_string(), CommandRiskTier::Sensitive);

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_exec = Arc::clone(&calls);
        let cmd = Command {
            id: "sensitive",
            summary: "test",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: Arc::new(move |_ctx, _args| {
                let calls_for_exec = Arc::clone(&calls_for_exec);
                Box::pin(async move {
                    calls_for_exec.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }),
        };

        struct NoopGate;
        #[async_trait::async_trait]
        impl BridgeGate for NoopGate {
            async fn before_execute(
                &self,
                _cmd: &Command,
                _args: &CommandArgs,
                _tier: CommandRiskTier,
            ) -> Result<(), BridgeError> {
                Ok(())
            }
        }

        // For MCP semantics, construct the bridge via `with_gate` (even if the gate is a no-op).
        let bridge =
            CommandAsToolBridge::new(policy).with_gate(Arc::new(NoopGate) as Arc<dyn BridgeGate>);
        let mut ctx = NoopCtx;
        bridge
            .invoke(
                &mut ctx,
                BridgeInvocation {
                    command: &cmd,
                    input: BridgeInput::Json(serde_json::json!({})),
                    confirmation: ConfirmationMode::NonInteractive,
                },
            )
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
