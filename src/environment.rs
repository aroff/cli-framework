//! Declarative registry for environment variables supported by an application.

use crate::command::CommandRegistry;
use crate::spec::EnvVarEntry;
use std::collections::BTreeMap;

/// Registry of environment variables exposed in root help.
///
/// Entries are sorted by name. Registering the same name and description more
/// than once is idempotent; conflicting descriptions are rejected.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentVariableRegistry {
    entries: BTreeMap<&'static str, &'static str>,
}

impl EnvironmentVariableRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare an environment variable supported by the application.
    pub fn register(&mut self, entry: EnvVarEntry) -> Result<(), EnvironmentVariableError> {
        if entry.name.trim().is_empty() {
            return Err(EnvironmentVariableError::EmptyName);
        }
        if entry.description.trim().is_empty() {
            return Err(EnvironmentVariableError::EmptyDescription {
                name: entry.name.to_string(),
            });
        }

        if let Some(existing) = self.entries.get(entry.name) {
            if *existing != entry.description {
                return Err(EnvironmentVariableError::ConflictingDescription {
                    name: entry.name.to_string(),
                    existing: (*existing).to_string(),
                    incoming: entry.description.to_string(),
                });
            }
            return Ok(());
        }

        self.entries.insert(entry.name, entry.description);
        Ok(())
    }

    /// Add environment declarations from every registered command.
    pub(crate) fn register_commands(
        &mut self,
        commands: &CommandRegistry,
    ) -> Result<(), EnvironmentVariableError> {
        for (_, command) in commands.all_tree_commands() {
            for entry in &command.spec.env_vars {
                self.register(entry.clone())?;
            }
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Iterate over declarations in stable name order.
    pub fn entries(&self) -> impl Iterator<Item = EnvVarEntry> + '_ {
        self.entries
            .iter()
            .map(|(name, description)| EnvVarEntry { name, description })
    }

    /// Render the section used by both framework and Clap root help.
    pub(crate) fn render_help(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let width = self
            .entries
            .keys()
            .map(|name| name.len())
            .max()
            .unwrap_or(0);
        let mut out = String::from("Environment Variables:\n");
        for (name, description) in &self.entries {
            out.push_str("  ");
            out.push_str(name);
            out.push_str(&" ".repeat(width.saturating_sub(name.len()) + 2));
            out.push_str(description);
            out.push('\n');
        }
        out
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvironmentVariableError {
    #[error("environment variable name must not be empty")]
    EmptyName,
    #[error("environment variable '{name}' must have a description")]
    EmptyDescription { name: String },
    #[error(
        "environment variable '{name}' has conflicting descriptions: '{existing}' and '{incoming}'"
    )]
    ConflictingDescription {
        name: String,
        existing: String,
        incoming: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_sorts_and_deduplicates_entries() {
        let mut registry = EnvironmentVariableRegistry::new();
        registry
            .register(EnvVarEntry {
                name: "Z_TOKEN",
                description: "Last token",
            })
            .unwrap();
        registry
            .register(EnvVarEntry {
                name: "A_TOKEN",
                description: "First token",
            })
            .unwrap();
        registry
            .register(EnvVarEntry {
                name: "A_TOKEN",
                description: "First token",
            })
            .unwrap();

        assert_eq!(registry.len(), 2);
        let names: Vec<_> = registry.entries().map(|entry| entry.name).collect();
        assert_eq!(names, ["A_TOKEN", "Z_TOKEN"]);
    }

    #[test]
    fn conflicting_descriptions_are_rejected() {
        let mut registry = EnvironmentVariableRegistry::new();
        registry
            .register(EnvVarEntry {
                name: "API_TOKEN",
                description: "Primary token",
            })
            .unwrap();

        let error = registry
            .register(EnvVarEntry {
                name: "API_TOKEN",
                description: "Fallback token",
            })
            .unwrap_err();

        assert!(matches!(
            error,
            EnvironmentVariableError::ConflictingDescription { .. }
        ));
    }
}
