use crate::security::{CommandRiskPolicy, CommandRiskTier};

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

    /// Shared risk-gate preflight used by `chat`.
    ///
    /// Contract: MUST preserve exact error messages and semantics.
    pub fn enforce_preflight(
        &self,
        command_id: &str,
        command_category: Option<&str>,
        assume_yes: bool,
        ailoop_available: bool,
    ) -> anyhow::Result<()> {
        let tier = self.policy.classify(command_id, command_category);
        match tier {
            CommandRiskTier::Safe => Ok(()),
            CommandRiskTier::Sensitive => {
                if !ailoop_available && !crate::cli_mode::is_interactive() && !assume_yes {
                    log::warn!(
                        "Sensitive command '{}' blocked in non-interactive mode without --yes",
                        command_id
                    );
                    return Err(anyhow::anyhow!(
                        "SENSITIVE_COMMAND_REQUIRES_CONFIRMATION: command '{}' is sensitive \
                         and requires interactive confirmation",
                        command_id
                    ));
                }
                Ok(())
            }
            CommandRiskTier::Destructive => {
                let env_allowed = std::env::var("ALLOW_DESTRUCTIVE_COMMANDS")
                    .map(|v| v == "1" || v == "true")
                    .unwrap_or(false);
                if !env_allowed {
                    log::warn!(
                        "Destructive command '{}' blocked: ALLOW_DESTRUCTIVE_COMMANDS not set",
                        command_id
                    );
                    return Err(anyhow::anyhow!(
                        "DESTRUCTIVE_COMMAND_BLOCKED: command '{}' is destructive; \
                         set ALLOW_DESTRUCTIVE_COMMANDS=1 and confirm interactively",
                        command_id
                    ));
                }
                if !ailoop_available && !crate::cli_mode::is_interactive() {
                    log::warn!(
                        "Destructive command '{}' blocked: non-interactive terminal",
                        command_id
                    );
                    return Err(anyhow::anyhow!(
                        "DESTRUCTIVE_COMMAND_BLOCKED: command '{}' requires an interactive \
                         terminal or ailoop when ALLOW_DESTRUCTIVE_COMMANDS=1",
                        command_id
                    ));
                }
                Ok(())
            }
        }
    }
}
