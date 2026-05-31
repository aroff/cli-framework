use crate::security::{CommandRiskPolicy, CommandRiskTier};
use std::fmt;

/// Typed reason a preflight check was blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefightError {
    /// Sensitive command requires interactive confirmation (non-interactive context, no ailoop).
    SensitiveNeedsConfirmation,
    /// Destructive command blocked because `ALLOW_DESTRUCTIVE_COMMANDS` is not set.
    DestructiveEnvGated,
    /// Destructive command blocked because the terminal is non-interactive and no ailoop is available.
    DestructiveNeedsInteractive,
}

impl fmt::Display for PrefightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrefightError::SensitiveNeedsConfirmation => write!(
                f,
                "SENSITIVE_COMMAND_REQUIRES_CONFIRMATION: command is sensitive \
                 and requires interactive confirmation"
            ),
            PrefightError::DestructiveEnvGated => write!(
                f,
                "DESTRUCTIVE_COMMAND_BLOCKED: command is destructive; \
                 set ALLOW_DESTRUCTIVE_COMMANDS=1 and confirm interactively"
            ),
            PrefightError::DestructiveNeedsInteractive => write!(
                f,
                "DESTRUCTIVE_COMMAND_BLOCKED: command requires an interactive \
                 terminal or ailoop when ALLOW_DESTRUCTIVE_COMMANDS=1"
            ),
        }
    }
}

impl std::error::Error for PrefightError {}

#[derive(Debug, Clone)]
pub struct RiskEnforcer {
    policy: CommandRiskPolicy,
}

impl RiskEnforcer {
    pub fn new(policy: CommandRiskPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &CommandRiskPolicy {
        &self.policy
    }

    pub fn classify(&self, command_id: &str, command_category: Option<&str>) -> CommandRiskTier {
        self.policy.classify(command_id, command_category)
    }

    /// Preflight gate used by `chat`.
    ///
    /// Returns a typed [`PrefightError`] rather than an opaque error so callers
    /// can match on the specific block reason without string parsing.  The
    /// `Display` impl preserves the legacy error message prefixes that golden
    /// tests assert against via `.to_string()`.
    pub fn enforce_preflight(
        &self,
        command_id: &str,
        command_category: Option<&str>,
        assume_yes: bool,
        ailoop_available: bool,
    ) -> Result<(), PrefightError> {
        let tier = self.policy.classify(command_id, command_category);
        match tier {
            CommandRiskTier::Safe => Ok(()),
            CommandRiskTier::Sensitive => {
                if !ailoop_available && !crate::cli_mode::is_interactive() && !assume_yes {
                    tracing::warn!(
                        "Sensitive command '{}' blocked in non-interactive mode without --yes",
                        command_id
                    );
                    return Err(PrefightError::SensitiveNeedsConfirmation);
                }
                Ok(())
            }
            CommandRiskTier::Destructive => {
                let env_allowed = std::env::var("ALLOW_DESTRUCTIVE_COMMANDS")
                    .map(|v| v == "1" || v == "true")
                    .unwrap_or(false);
                if !env_allowed {
                    tracing::warn!(
                        "Destructive command '{}' blocked: ALLOW_DESTRUCTIVE_COMMANDS not set",
                        command_id
                    );
                    return Err(PrefightError::DestructiveEnvGated);
                }
                if !ailoop_available && !crate::cli_mode::is_interactive() {
                    tracing::warn!(
                        "Destructive command '{}' blocked: non-interactive terminal",
                        command_id
                    );
                    return Err(PrefightError::DestructiveNeedsInteractive);
                }
                Ok(())
            }
        }
    }
}
