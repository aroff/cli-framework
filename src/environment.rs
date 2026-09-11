//! Declarative registry for environment variables supported by an application.

use crate::command::CommandRegistry;
use crate::spec::EnvVarEntry;
use std::borrow::Cow;
use std::collections::BTreeMap;

/// Registry of environment variables exposed in root help.
///
/// Entries are sorted by name. Registering the same name and description more
/// than once is idempotent; conflicting descriptions are rejected.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentVariableRegistry {
    entries: BTreeMap<Cow<'static, str>, Cow<'static, str>>,
}

impl EnvironmentVariableRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare an environment variable supported by the application.
    pub fn register(&mut self, entry: EnvVarEntry) -> Result<(), EnvironmentVariableError> {
        self.insert(
            Cow::Borrowed(entry.name),
            Cow::Borrowed(entry.description),
            true,
        )
        .map(|_| ())
    }

    /// Declare a variable the *framework* honours, without displacing the
    /// application's own declaration of the same name.
    ///
    /// The framework's telemetry variables are computed from the app name at
    /// build time, so they arrive as owned strings rather than the `'static`
    /// literals of [`EnvVarEntry`]. An application that has already
    /// registered the name keeps its own description — its author knows more
    /// about how the app uses the variable than the framework does — so this
    /// never reports [`EnvironmentVariableError::ConflictingDescription`].
    /// Returns whether a new entry was added.
    pub fn register_if_absent(
        &mut self,
        name: impl Into<Cow<'static, str>>,
        description: impl Into<Cow<'static, str>>,
    ) -> Result<bool, EnvironmentVariableError> {
        self.insert(name.into(), description.into(), false)
    }

    fn insert(
        &mut self,
        name: Cow<'static, str>,
        description: Cow<'static, str>,
        reject_conflict: bool,
    ) -> Result<bool, EnvironmentVariableError> {
        if name.trim().is_empty() {
            return Err(EnvironmentVariableError::EmptyName);
        }
        if description.trim().is_empty() {
            return Err(EnvironmentVariableError::EmptyDescription {
                name: name.into_owned(),
            });
        }

        if let Some(existing) = self.entries.get(name.as_ref()) {
            if reject_conflict && *existing != description {
                return Err(EnvironmentVariableError::ConflictingDescription {
                    name: name.into_owned(),
                    existing: existing.to_string(),
                    incoming: description.into_owned(),
                });
            }
            return Ok(false);
        }

        self.entries.insert(name, description);
        Ok(true)
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

    /// Iterate over `(name, description)` pairs in stable name order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.entries
            .iter()
            .map(|(name, description)| (name.as_ref(), description.as_ref()))
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
        let names: Vec<_> = registry.entries().map(|(name, _)| name).collect();
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

    #[test]
    fn register_if_absent_keeps_the_existing_description() {
        let mut registry = EnvironmentVariableRegistry::new();
        registry
            .register(EnvVarEntry {
                name: "APP_TELEMETRY_LEVEL",
                description: "The app's own wording",
            })
            .unwrap();

        let added = registry
            .register_if_absent(
                "APP_TELEMETRY_LEVEL".to_string(),
                "Framework wording".to_string(),
            )
            .unwrap();

        assert!(
            !added,
            "an existing name must be left alone, not reported as added"
        );
        assert_eq!(registry.len(), 1);
        let (_, description) = registry.entries().next().unwrap();
        assert_eq!(description, "The app's own wording");
    }

    #[test]
    fn register_if_absent_adds_owned_names_and_renders_them() {
        let mut registry = EnvironmentVariableRegistry::new();
        let added = registry
            .register_if_absent(format!("{}_HOME", "DEMO"), "Data directory")
            .unwrap();

        assert!(added);
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry.render_help(),
            "Environment Variables:\n  DEMO_HOME  Data directory\n"
        );
    }

    #[test]
    fn register_if_absent_validates_like_register() {
        let mut registry = EnvironmentVariableRegistry::new();
        assert_eq!(
            registry.register_if_absent("  ", "x").unwrap_err(),
            EnvironmentVariableError::EmptyName
        );
        assert_eq!(
            registry.register_if_absent("NAME", " ").unwrap_err(),
            EnvironmentVariableError::EmptyDescription {
                name: "NAME".to_string()
            }
        );
        assert!(registry.is_empty());
    }
}
