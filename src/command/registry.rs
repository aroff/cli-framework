//! CommandRegistry — tree-backed command storage with O(1) path lookup.

use crate::command::Command;
use crate::parser::error_codes::{E_ALIAS_CONFLICT, E_REGISTRATION_COLLISION};
use crate::spec::command_tree::{CommandPath, GroupMetadata};
use std::collections::HashMap;

// ── RegistrationError ─────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("[{code}] command path '{path}' is already occupied", code = E_REGISTRATION_COLLISION)]
    Collision { path: String },

    #[error("[{code}] alias '{alias}' conflicts with existing path '{existing_path}'", code = E_ALIAS_CONFLICT)]
    AliasConflict {
        alias: String,
        existing_path: String,
    },
}

// ── CommandRegistry ───────────────────────────────────────────────────────────

/// Registry for managing commands. Stores all commands keyed by their full
/// path string (e.g. `"mcp/serve"`). Root-level commands have a single-segment
/// key identical to their id, so flat `get(id)` is just a path lookup.
#[derive(Clone)]
pub struct CommandRegistry {
    /// All registered commands by full path string (e.g. "cluster/get", "deploy").
    tree_commands: HashMap<String, Command>,
    /// Group metadata by path string (non-leaf group nodes).
    groups: HashMap<String, GroupMetadata>,
    /// Consumer-defined ordering for categorized root-help sections.
    help_section_order: Vec<String>,
    /// Exact registry path of the framework-provided completion command.
    ///
    /// Consumer commands may also use `completion` as a leaf id, so tool
    /// surfaces must use this identity instead of filtering by name.
    framework_completion_path: Option<String>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            tree_commands: HashMap::new(),
            groups: HashMap::new(),
            help_section_order: Vec::new(),
            framework_completion_path: None,
        }
    }

    // ── Flat API ──────────────────────────────────────────────────────────────

    /// Register a command at the root level (flat, backward-compatible).
    ///
    /// Panics on collision; prefer `AppBuilder::register_command()` which
    /// surfaces the error as `Result`.
    pub fn register(&mut self, command: Command) {
        let path = CommandPath::root_for(&command.id);
        self.register_at(&path, command)
            .expect("command registration collision");
    }

    /// Get a command by its root-level id (single-segment path).
    pub fn get(&self, id: &str) -> Option<&Command> {
        self.tree_commands.get(id)
    }

    /// Iterate over all root-level (single-segment path) commands.
    pub fn commands(&self) -> impl Iterator<Item = &Command> {
        self.tree_commands
            .iter()
            .filter(|(k, _)| !k.contains('/'))
            .map(|(_, v)| v)
    }

    // ── Tree API ──────────────────────────────────────────────────────────────

    /// Register a group node (no command, just metadata).
    pub fn register_group(
        &mut self,
        path: &CommandPath,
        metadata: GroupMetadata,
    ) -> Result<(), RegistrationError> {
        let path_str = path.to_path_string();
        if self.tree_commands.contains_key(&path_str)
            || self.groups.contains_key(&path_str)
            || self.command_ancestor(path).is_some()
        {
            return Err(RegistrationError::Collision { path: path_str });
        }
        if let Some((alias, existing_path)) = self
            .existing_alias_for_path(&path_str)
            .or_else(|| self.existing_alias_ancestor(path))
        {
            return Err(RegistrationError::AliasConflict {
                alias,
                existing_path,
            });
        }
        self.groups.insert(path_str, metadata);
        Ok(())
    }

    /// Look up group metadata by path string (e.g., `"mcp"`).
    pub fn group_metadata_for(&self, path_str: &str) -> Option<&GroupMetadata> {
        self.groups.get(path_str)
    }

    /// Iterate over all registered group nodes.
    pub fn groups(&self) -> impl Iterator<Item = (&str, &GroupMetadata)> {
        self.groups.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Set the preferred root-help section order.
    ///
    /// Section labels not present here continue to render alphabetically after
    /// the explicitly ordered sections. Duplicate labels keep their first
    /// position.
    pub fn set_help_section_order(&mut self, sections: &[&str]) {
        self.help_section_order.clear();
        for section in sections {
            if !self.help_section_order.iter().any(|known| known == section) {
                self.help_section_order.push((*section).to_string());
            }
        }
    }

    /// Return the consumer-defined root-help section order.
    pub fn help_section_order(&self) -> &[String] {
        &self.help_section_order
    }

    /// Return the explicit rank of a root-help section, if configured.
    pub fn help_section_rank(&self, section: &str) -> Option<usize> {
        self.help_section_order
            .iter()
            .position(|known| known == section)
    }

    /// Register a command at a specific `CommandPath`.
    ///
    /// Returns `Err(RegistrationError::Collision)` if the path is already occupied.
    /// Returns `Err(RegistrationError::AliasConflict)` if any alias in the command's
    /// `CommandSpec` collides with an existing path or alias.
    pub fn register_at(
        &mut self,
        path: &CommandPath,
        command: Command,
    ) -> Result<(), RegistrationError> {
        let path_str = path.to_path_string();

        if self.tree_commands.contains_key(&path_str)
            || self.groups.contains_key(&path_str)
            || self.command_ancestor(path).is_some()
            || self.path_has_descendants(&path_str)
        {
            return Err(RegistrationError::Collision { path: path_str });
        }

        if let Some((alias, existing_path)) = self.existing_alias_for_path(&path_str) {
            return Err(RegistrationError::AliasConflict {
                alias,
                existing_path,
            });
        }
        if let Some((alias, existing_path)) = self.existing_alias_ancestor(path) {
            return Err(RegistrationError::AliasConflict {
                alias,
                existing_path,
            });
        }

        let spec = &command.spec;
        for alias in spec.aliases.iter().chain(spec.hidden_aliases.iter()) {
            let alias_path = qualified_alias_path(path, alias);
            if let Some((existing_key, _)) = self.tree_commands.get_key_value(&alias_path) {
                return Err(RegistrationError::AliasConflict {
                    alias: alias.to_string(),
                    existing_path: existing_key.clone(),
                });
            }
            if self.groups.contains_key(&alias_path) {
                return Err(RegistrationError::AliasConflict {
                    alias: alias.to_string(),
                    existing_path: alias_path,
                });
            }
            if self.path_has_descendants(&alias_path) {
                return Err(RegistrationError::AliasConflict {
                    alias: alias.to_string(),
                    existing_path: alias_path,
                });
            }
            if let Some((_, existing_path)) = self.existing_alias_for_path(&alias_path) {
                return Err(RegistrationError::AliasConflict {
                    alias: alias.to_string(),
                    existing_path,
                });
            }
        }

        self.tree_commands.insert(path_str, command);
        Ok(())
    }

    /// Register and identify the framework-provided completion command.
    pub(crate) fn register_framework_completion_at(
        &mut self,
        path: &CommandPath,
        command: Command,
    ) -> Result<(), RegistrationError> {
        self.register_at(path, command)?;
        self.framework_completion_path = Some(path.to_path_string());
        Ok(())
    }

    /// Return whether `path` identifies the framework-provided completion
    /// command rather than a consumer command with the same leaf id.
    pub(crate) fn is_framework_completion(&self, path: &str) -> bool {
        self.framework_completion_path.as_deref() == Some(path)
    }

    /// Resolve a command by `CommandPath`.
    pub fn resolve(&self, path: &CommandPath) -> Option<&Command> {
        self.tree_commands.get(&path.to_path_string())
    }

    /// Iterate over all commands in the tree (including hierarchical), with their path strings.
    pub fn all_tree_commands(&self) -> impl Iterator<Item = (&str, &Command)> {
        self.tree_commands.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// List direct child paths of the given path.
    pub fn list_children(&self, path: &CommandPath) -> Vec<CommandPath> {
        let prefix = if path.0.is_empty() {
            String::new()
        } else {
            format!("{}/", path.to_path_string())
        };

        self.tree_commands
            .keys()
            .filter_map(|key| {
                if key.starts_with(&prefix) {
                    let rest = &key[prefix.len()..];
                    if !rest.contains('/') && !rest.is_empty() {
                        let mut segments = path.0.clone();
                        segments.push(rest.to_string());
                        Some(CommandPath(segments))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    fn command_ancestor(&self, path: &CommandPath) -> Option<String> {
        (1..path.0.len()).find_map(|len| {
            let ancestor = path.0[..len].join("/");
            self.tree_commands
                .contains_key(&ancestor)
                .then_some(ancestor)
        })
    }

    fn path_has_descendants(&self, path: &str) -> bool {
        let prefix = format!("{path}/");
        self.tree_commands
            .keys()
            .any(|key| key.starts_with(&prefix))
            || self.groups.keys().any(|key| key.starts_with(&prefix))
    }

    fn existing_alias_for_path(&self, candidate: &str) -> Option<(String, String)> {
        self.tree_commands
            .iter()
            .find_map(|(existing_path, existing_command)| {
                existing_command
                    .spec
                    .aliases
                    .iter()
                    .chain(existing_command.spec.hidden_aliases.iter())
                    .find(|alias| {
                        qualified_alias_path(
                            &CommandPath(existing_path.split('/').map(str::to_string).collect()),
                            alias,
                        ) == candidate
                    })
                    .map(|alias| ((*alias).to_string(), existing_path.clone()))
            })
    }

    fn existing_alias_ancestor(&self, path: &CommandPath) -> Option<(String, String)> {
        (1..path.0.len()).find_map(|len| {
            let ancestor = path.0[..len].join("/");
            self.existing_alias_for_path(&ancestor)
        })
    }
}

fn qualified_alias_path(command_path: &CommandPath, alias: &str) -> String {
    if alias.contains('/') {
        return alias.to_string();
    }

    let mut segments = command_path.0[..command_path.0.len().saturating_sub(1)].to_vec();
    segments.push(alias.to_string());
    segments.join("/")
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// The label a `cli.command` probe attaches for this path, if and only if the
/// registry actually declares it.
///
/// `None` for anything the registry does not resolve — a typo, a plugin that
/// failed to load, an injected argument — so a caller building metric labels
/// never has to decide separately whether the path is trustworthy: an
/// unvalidated path is unbounded cardinality and a potential leak, so it must
/// never reach a label.
pub fn registered_command_label(registry: &CommandRegistry, path: &[String]) -> Option<String> {
    let command_path = CommandPath(path.to_vec());
    registry.resolve(&command_path)?;
    Some(path.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::command_tree::CommandSpec;
    use std::sync::Arc;

    fn make_cmd(id: &'static str) -> Command {
        Command {
            id: Arc::from(id),
            spec: Arc::new(CommandSpec::default()),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
        }
    }

    #[test]
    fn e007_collision_on_re_register() {
        let mut registry = CommandRegistry::new();
        let path = CommandPath::new(&["cluster", "get"]).unwrap();
        registry.register_at(&path, make_cmd("get")).unwrap();
        let err = registry.register_at(&path, make_cmd("get")).unwrap_err();
        match err {
            RegistrationError::Collision { path } => assert_eq!(path, "cluster/get"),
            _ => panic!("expected Collision"),
        }
    }

    #[test]
    fn e008_alias_conflict() {
        let mut registry = CommandRegistry::new();
        registry.register(make_cmd("hello"));

        let cmd = Command {
            id: Arc::from("greet"),
            spec: Arc::new(CommandSpec {
                aliases: vec!["hello"],
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
        };

        let err = registry
            .register_at(&CommandPath::root_for("greet"), cmd)
            .unwrap_err();
        match err {
            RegistrationError::AliasConflict {
                alias,
                existing_path,
            } => {
                assert_eq!(alias, "hello");
                assert_eq!(existing_path, "hello");
            }
            _ => panic!("expected AliasConflict"),
        }
    }

    #[test]
    fn e008_alias_conflict_reports_full_nested_path() {
        // Register a nested command: map key is "mcp/serve", Command.id is "serve".
        // Then register another command using alias "mcp/serve" (the full path string).
        // Before fix: existing_path = "serve" (Command.id leaf).
        // After fix: existing_path = "mcp/serve" (full map key).
        let mut registry = CommandRegistry::new();
        let path = CommandPath::new(&["mcp", "serve"]).unwrap();
        registry.register_at(&path, make_cmd("serve")).unwrap();

        let cmd = Command {
            id: Arc::from("other"),
            spec: Arc::new(CommandSpec {
                aliases: vec!["mcp/serve"],
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
        };

        let err = registry
            .register_at(&CommandPath::root_for("other"), cmd)
            .unwrap_err();
        match err {
            RegistrationError::AliasConflict {
                alias,
                existing_path,
            } => {
                assert_eq!(alias, "mcp/serve");
                assert_eq!(existing_path, "mcp/serve");
            }
            _ => panic!("expected AliasConflict"),
        }
    }

    #[test]
    fn e008_alias_conflict_includes_hidden_aliases() {
        let mut registry = CommandRegistry::new();
        registry.register(make_cmd("hello"));

        let cmd = Command {
            id: Arc::from("greet"),
            spec: Arc::new(CommandSpec {
                hidden_aliases: vec!["hello"],
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
        };

        let err = registry
            .register_at(&CommandPath::root_for("greet"), cmd)
            .unwrap_err();
        match err {
            RegistrationError::AliasConflict {
                alias,
                existing_path,
            } => {
                assert_eq!(alias, "hello");
                assert_eq!(existing_path, "hello");
            }
            _ => panic!("expected AliasConflict"),
        }
    }

    #[test]
    fn nested_aliases_are_qualified_relative_to_the_command_parent() {
        let mut registry = CommandRegistry::new();
        registry
            .register_at(
                &CommandPath::new(&["cli", "completions"]).unwrap(),
                make_cmd("completions"),
            )
            .unwrap();
        let command = Command {
            id: Arc::from("completion"),
            spec: Arc::new(CommandSpec {
                hidden_aliases: vec!["completions"],
                ..Default::default()
            }),
            ..make_cmd("completion")
        };

        let error = registry
            .register_at(&CommandPath::new(&["cli", "completion"]).unwrap(), command)
            .unwrap_err();

        assert!(matches!(
            error,
            RegistrationError::AliasConflict {
                alias,
                existing_path
            } if alias == "completions" && existing_path == "cli/completions"
        ));
    }

    #[test]
    fn canonical_path_conflicts_with_an_existing_sibling_alias() {
        let mut registry = CommandRegistry::new();
        let command = Command {
            id: Arc::from("completion"),
            spec: Arc::new(CommandSpec {
                hidden_aliases: vec!["completions"],
                ..Default::default()
            }),
            ..make_cmd("completion")
        };
        registry
            .register_at(&CommandPath::new(&["cli", "completion"]).unwrap(), command)
            .unwrap();

        let error = registry
            .register_at(
                &CommandPath::new(&["cli", "completions"]).unwrap(),
                make_cmd("completions"),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            RegistrationError::AliasConflict {
                alias,
                existing_path
            } if alias == "completions" && existing_path == "cli/completion"
        ));
    }

    #[test]
    fn command_paths_cannot_be_ancestors_or_descendants_of_other_commands() {
        let mut descendant_first = CommandRegistry::new();
        descendant_first
            .register_at(
                &CommandPath::new(&["cli", "spec"]).unwrap(),
                make_cmd("spec"),
            )
            .unwrap();
        assert!(matches!(
            descendant_first.register_at(&CommandPath::root_for("cli"), make_cmd("cli")),
            Err(RegistrationError::Collision { path }) if path == "cli"
        ));

        let mut ancestor_first = CommandRegistry::new();
        ancestor_first.register(make_cmd("cli"));
        assert!(matches!(
            ancestor_first.register_at(
                &CommandPath::new(&["cli", "spec"]).unwrap(),
                make_cmd("spec")
            ),
            Err(RegistrationError::Collision { path }) if path == "cli/spec"
        ));
    }

    #[test]
    fn groups_and_command_aliases_cannot_occupy_the_same_path() {
        let mut alias_first = CommandRegistry::new();
        let owner = Command {
            id: Arc::from("owner"),
            spec: Arc::new(CommandSpec {
                aliases: vec!["cli"],
                ..Default::default()
            }),
            ..make_cmd("owner")
        };
        alias_first.register(owner);
        assert!(matches!(
            alias_first.register_group(&CommandPath::root_for("cli"), GroupMetadata::default()),
            Err(RegistrationError::AliasConflict { .. })
        ));

        let mut group_first = CommandRegistry::new();
        group_first
            .register_group(
                &CommandPath::new(&["cli", "completions"]).unwrap(),
                GroupMetadata::default(),
            )
            .unwrap();
        let completion = Command {
            id: Arc::from("completion"),
            spec: Arc::new(CommandSpec {
                hidden_aliases: vec!["completions"],
                ..Default::default()
            }),
            ..make_cmd("completion")
        };
        assert!(matches!(
            group_first.register_at(
                &CommandPath::new(&["cli", "completion"]).unwrap(),
                completion
            ),
            Err(RegistrationError::AliasConflict { .. })
        ));
    }

    #[test]
    fn aliases_cannot_shadow_implicit_groups_or_other_aliases() {
        let mut implicit_group = CommandRegistry::new();
        implicit_group
            .register_at(
                &CommandPath::new(&["cli", "completions", "show"]).unwrap(),
                make_cmd("show"),
            )
            .unwrap();
        let completion = Command {
            id: Arc::from("completion"),
            spec: Arc::new(CommandSpec {
                hidden_aliases: vec!["completions"],
                ..Default::default()
            }),
            ..make_cmd("completion")
        };
        assert!(matches!(
            implicit_group.register_at(
                &CommandPath::new(&["cli", "completion"]).unwrap(),
                completion
            ),
            Err(RegistrationError::AliasConflict { .. })
        ));

        let mut alias_collision = CommandRegistry::new();
        let first = Command {
            id: Arc::from("first"),
            spec: Arc::new(CommandSpec {
                aliases: vec!["shared"],
                ..Default::default()
            }),
            ..make_cmd("first")
        };
        alias_collision
            .register_at(&CommandPath::new(&["cli", "first"]).unwrap(), first)
            .unwrap();
        let second = Command {
            id: Arc::from("second"),
            spec: Arc::new(CommandSpec {
                aliases: vec!["shared"],
                ..Default::default()
            }),
            ..make_cmd("second")
        };
        assert!(matches!(
            alias_collision.register_at(&CommandPath::new(&["cli", "second"]).unwrap(), second),
            Err(RegistrationError::AliasConflict { .. })
        ));
    }

    #[test]
    fn register_two_level_path_and_resolve() {
        let mut registry = CommandRegistry::new();
        let path = CommandPath::new(&["cluster", "get"]).unwrap();
        registry.register_at(&path, make_cmd("get")).unwrap();
        let found = registry.resolve(&path);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id.as_ref(), "get");
    }

    #[test]
    fn resolve_missing_path_returns_none() {
        let registry = CommandRegistry::new();
        let path = CommandPath::new(&["missing"]).unwrap();
        assert!(registry.resolve(&path).is_none());
    }

    #[test]
    fn legacy_get_works_after_register_at_root() {
        let mut registry = CommandRegistry::new();
        registry
            .register_at(&CommandPath::root_for("hello"), make_cmd("hello"))
            .unwrap();
        assert!(registry.get("hello").is_some());
    }

    #[test]
    fn get_does_not_match_nested_by_leaf_id() {
        let mut registry = CommandRegistry::new();
        registry
            .register_at(
                &CommandPath::new(&["mcp", "serve"]).unwrap(),
                make_cmd("serve"),
            )
            .unwrap();
        // flat get("serve") must NOT match a nested path
        assert!(registry.get("serve").is_none());
        assert!(registry
            .resolve(&CommandPath::new(&["mcp", "serve"]).unwrap())
            .is_some());
    }

    #[test]
    fn commands_iterator_only_returns_root_level() {
        let mut registry = CommandRegistry::new();
        registry.register(make_cmd("deploy"));
        registry
            .register_at(
                &CommandPath::new(&["mcp", "serve"]).unwrap(),
                make_cmd("serve"),
            )
            .unwrap();
        let root_ids: Vec<_> = registry.commands().map(|c| c.id.as_ref()).collect();
        assert!(root_ids.contains(&"deploy"));
        assert!(
            !root_ids.contains(&"serve"),
            "nested command must not appear in root iterator"
        );
    }

    #[test]
    fn list_children_returns_direct_children() {
        let mut registry = CommandRegistry::new();
        registry
            .register_at(
                &CommandPath::new(&["cluster", "get"]).unwrap(),
                make_cmd("get"),
            )
            .unwrap();
        registry
            .register_at(
                &CommandPath::new(&["cluster", "list"]).unwrap(),
                make_cmd("list"),
            )
            .unwrap();

        let parent = CommandPath::root_for("cluster");
        let mut children = registry.list_children(&parent);
        children.sort_by_key(|p| p.to_path_string());
        assert_eq!(
            children
                .iter()
                .map(|p| p.to_path_string())
                .collect::<Vec<_>>(),
            vec!["cluster/get", "cluster/list"]
        );
    }

    #[test]
    fn legacy_register_works_and_get_returns_command() {
        let mut registry = CommandRegistry::new();
        registry.register(make_cmd("deploy"));
        assert!(registry.get("deploy").is_some());
    }

    #[test]
    fn registered_command_label_is_some_for_a_path_the_registry_resolves() {
        let mut registry = CommandRegistry::new();
        registry
            .register_at(
                &CommandPath::new(&["cluster", "get"]).unwrap(),
                make_cmd("get"),
            )
            .unwrap();
        let path = vec!["cluster".to_string(), "get".to_string()];
        assert_eq!(
            registered_command_label(&registry, &path),
            Some("cluster get".to_string())
        );
    }

    #[test]
    fn registered_command_label_is_none_for_a_path_the_registry_does_not_declare() {
        let registry = CommandRegistry::new();
        let path = vec!["not".to_string(), "registered".to_string()];
        assert_eq!(registered_command_label(&registry, &path), None);
    }
}
