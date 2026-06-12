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
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            tree_commands: HashMap::new(),
            groups: HashMap::new(),
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
        if self.tree_commands.contains_key(&path_str) || self.groups.contains_key(&path_str) {
            return Err(RegistrationError::Collision { path: path_str });
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

        if self.tree_commands.contains_key(&path_str) || self.groups.contains_key(&path_str) {
            return Err(RegistrationError::Collision { path: path_str });
        }

        let spec = &command.spec;
        for alias in spec.aliases.iter().chain(spec.hidden_aliases.iter()) {
            if let Some((existing_key, _)) = self.tree_commands.get_key_value(*alias) {
                return Err(RegistrationError::AliasConflict {
                    alias: alias.to_string(),
                    existing_path: existing_key.clone(),
                });
            }
        }

        self.tree_commands.insert(path_str, command);
        Ok(())
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
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
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
            ui: None,
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
            ui: None,
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
            ui: None,
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
            ui: None,
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
}
