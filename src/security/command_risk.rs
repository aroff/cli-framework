use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandRiskTier {
    Safe,
    Sensitive,
    Destructive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRiskPolicy {
    /// Per-command tier overrides, keyed by Command.id.
    pub tiers: HashMap<String, CommandRiskTier>,
    /// Tier assigned when no per-command override and no matching category rule.
    pub default_tier: CommandRiskTier,
}

impl Default for CommandRiskPolicy {
    fn default() -> Self {
        Self {
            tiers: HashMap::new(),
            default_tier: CommandRiskTier::Safe,
        }
    }
}

impl CommandRiskPolicy {
    /// Classify a command by ID and optional category.
    ///
    /// Priority: per-command override > category rule > default_tier.
    pub fn classify(&self, command_id: &str, command_category: Option<&str>) -> CommandRiskTier {
        if let Some(&tier) = self.tiers.get(command_id) {
            return tier;
        }
        match command_category {
            Some("deployment") | Some("admin") | Some("destructive") => {
                CommandRiskTier::Destructive
            }
            Some("data") | Some("config") => CommandRiskTier::Sensitive,
            _ => self.default_tier,
        }
    }
}
