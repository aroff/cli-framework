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

const USER_DECLINED_CONFIRMATION_PREFIX: &str = "USER_DECLINED_CONFIRMATION:";

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
}

impl CommandAsToolBridge {
    pub fn new(risk_policy: CommandRiskPolicy) -> Self {
        Self {
            risk_policy,
            gate: None,
        }
    }

    pub fn with_gate(mut self, gate: Arc<dyn BridgeGate>) -> Self {
        self.gate = Some(gate);
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
        _command: &Command,
        input: BridgeInput,
    ) -> Result<ParsedArgs, BridgeError> {
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
        self.invoke_inner(ctx, invocation).await
    }

    async fn invoke_inner(
        &self,
        ctx: &mut dyn AppContext,
        invocation: BridgeInvocation<'_>,
    ) -> Result<(), BridgeError> {
        let cmd = invocation.command;
        let is_mcp_surface = self.gate.is_some()
            && matches!(invocation.confirmation, ConfirmationMode::NonInteractive);

        let parsed = self.parse_args(cmd, invocation.input)?;

        if let Some(ref typed) = parsed.typed {
            let should_validate = if is_mcp_surface {
                // Preserve MCP behavior: typed validation only runs when `spec.is_some()`.
                cmd.spec.is_some()
            } else {
                // Preserve chat behavior: validate when `spec` or custom `validator` exists.
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

        if !is_mcp_surface {
            let assume_yes = matches!(invocation.confirmation, ConfirmationMode::AssumeYes);
            let ailoop_available = matches!(invocation.confirmation, ConfirmationMode::Ailoop(_));
            if let Err(e) =
                enforcer.enforce_preflight(cmd.id, cmd.category, assume_yes, ailoop_available)
            {
                let msg = e.to_string();
                if msg.starts_with("SENSITIVE_COMMAND_REQUIRES_CONFIRMATION:") {
                    return Err(BridgeError::SensitiveRequiresConfirmation(
                        cmd.id.to_string(),
                    ));
                }
                if msg.starts_with("DESTRUCTIVE_COMMAND_BLOCKED:") {
                    return Err(BridgeError::DestructiveBlocked(cmd.id.to_string()));
                }
                return Err(BridgeError::Execution(e));
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
            } else {
                Err(BridgeError::Execution(anyhow::anyhow!(
                    "{USER_DECLINED_CONFIRMATION_PREFIX}{}:{}",
                    if destructive {
                        "destructive"
                    } else {
                        "sensitive"
                    },
                    cmd.id
                )))
            }
        }
        ConfirmationMode::InteractiveStdin => {
            let confirmed = prompt_confirm_blocking(prompt)
                .await
                .map_err(BridgeError::Execution)?;
            if confirmed {
                Ok(())
            } else {
                Err(BridgeError::Execution(anyhow::anyhow!(
                    "{USER_DECLINED_CONFIRMATION_PREFIX}{}:{}",
                    if destructive {
                        "destructive"
                    } else {
                        "sensitive"
                    },
                    cmd.id
                )))
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

pub(crate) fn is_user_declined_confirmation_error(
    err: &anyhow::Error,
) -> Option<(DeclinedConfirmationKind, String)> {
    let msg = err.to_string();
    let rest = msg.strip_prefix(USER_DECLINED_CONFIRMATION_PREFIX)?;
    let (kind, cmd_id) = rest.split_once(':')?;
    if cmd_id.is_empty() {
        return None;
    }
    match kind {
        "sensitive" => Some((DeclinedConfirmationKind::Sensitive, cmd_id.to_string())),
        "destructive" => Some((DeclinedConfirmationKind::Destructive, cmd_id.to_string())),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclinedConfirmationKind {
    Sensitive,
    Destructive,
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

        let bridge = CommandAsToolBridge::new(policy).with_gate(Arc::new(NoopGate));
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

    #[test]
    fn user_declined_confirmation_marker_is_detected() {
        let err = anyhow::anyhow!("{USER_DECLINED_CONFIRMATION_PREFIX}sensitive:mycmd");
        let (kind, cmd_id) = is_user_declined_confirmation_error(&err).expect("marker detected");
        assert_eq!(kind, DeclinedConfirmationKind::Sensitive);
        assert_eq!(cmd_id, "mycmd");
    }
}
