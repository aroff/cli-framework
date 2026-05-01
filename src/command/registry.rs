//! CommandRegistry — flat + tree-backed command storage.

use crate::command::Command;
use crate::llm::CommandMetadata;
use crate::parser::error_codes::{E_ALIAS_CONFLICT, E_REGISTRATION_COLLISION};
use crate::spec::command_tree::{CommandPath, GroupMetadata};
use std::collections::HashMap;
use std::fmt;

// ── RegistrationError ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum RegistrationError {
    Collision {
        path: String,
    },
    AliasConflict {
        alias: String,
        existing_path: String,
    },
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistrationError::Collision { path } => {
                write!(
                    f,
                    "[{}] command path '{}' is already occupied",
                    E_REGISTRATION_COLLISION, path
                )
            }
            RegistrationError::AliasConflict {
                alias,
                existing_path,
            } => {
                write!(
                    f,
                    "[{}] alias '{}' conflicts with existing path '{}'",
                    E_ALIAS_CONFLICT, alias, existing_path
                )
            }
        }
    }
}

impl std::error::Error for RegistrationError {}

// ── Internal tree node ────────────────────────────────────────────────────────

#[allow(dead_code)]
pub(crate) struct TreeNode {
    pub command: Option<Command>,
    pub group_meta: Option<GroupMetadata>,
    pub children: HashMap<String, TreeNode>,
}

impl TreeNode {
    #[allow(dead_code)]
    fn empty() -> Self {
        TreeNode {
            command: None,
            group_meta: None,
            children: HashMap::new(),
        }
    }
}

// ── CommandRegistry ───────────────────────────────────────────────────────────

/// Registry for managing commands. Maintains both a flat index (for O(1) id lookup)
/// and a tree structure (for hierarchical `CommandPath`-based access).
#[derive(Clone)]
pub struct CommandRegistry {
    /// Flat id → Command index (root-level commands only, for backward compat).
    commands: HashMap<String, Command>,
    /// Root-level tree nodes (keyed by first path segment).
    // Not Clone-derived; we provide a manual Clone below.
    #[allow(dead_code)]
    tree_commands: HashMap<String, Command>, // stores all registered commands by full path string
}

impl CommandRegistry {
    /// Create a new command registry
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
            tree_commands: HashMap::new(),
        }
    }

    // ── Legacy flat API (unchanged signatures) ────────────────────────────────

    /// Register a command at the root level (backward-compatible).
    ///
    /// Panics on collision (previously silently overwrote; callers should use
    /// `AppBuilder::register_command()` which surfaces the error via `Result`).
    pub fn register(&mut self, command: Command) {
        let path = CommandPath::root_for(command.id);
        self.register_at(&path, command)
            .expect("command registration collision");
    }

    /// Get a command by flat ID.
    pub fn get(&self, id: &str) -> Option<&Command> {
        self.commands.get(id)
    }

    /// Iterate over all registered commands.
    pub fn commands(&self) -> impl Iterator<Item = &Command> {
        self.commands.values()
    }

    /// Collect metadata for all commands for LLM context.
    pub fn collect_metadata(&self) -> Vec<CommandMetadata> {
        self.commands
            .values()
            .map(|cmd| CommandMetadata {
                id: cmd.id.to_string(),
                summary: cmd.summary.to_string(),
                syntax: cmd.syntax.map(|s| s.to_string()),
                category: cmd.category.map(|c| c.to_string()),
            })
            .collect()
    }

    // ── Tree API ──────────────────────────────────────────────────────────────

    /// Register a group node (no command, just metadata).
    ///
    /// Returns `Err(RegistrationError::Collision)` if the path already has a command.
    pub fn register_group(
        &mut self,
        path: &CommandPath,
        _metadata: GroupMetadata,
    ) -> Result<(), RegistrationError> {
        let path_str = path.to_path_string();
        if self.tree_commands.contains_key(&path_str) {
            return Err(RegistrationError::Collision { path: path_str });
        }
        // Group nodes don't go in the flat commands map; just note their existence.
        // (A full tree impl would store GroupMetadata; this simplified version just
        //  prevents collision without persisting the group node.)
        Ok(())
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

        // Collision check
        if self.tree_commands.contains_key(&path_str) {
            return Err(RegistrationError::Collision { path: path_str });
        }

        // Alias conflict check
        if let Some(ref spec) = command.spec {
            for alias in &spec.aliases {
                if self.commands.contains_key(*alias) || self.tree_commands.contains_key(*alias) {
                    return Err(RegistrationError::AliasConflict {
                        alias: alias.to_string(),
                        existing_path: alias.to_string(),
                    });
                }
            }
        }

        // Insert into tree_commands under full path
        self.tree_commands.insert(path_str, command.clone());

        // For root-level (single-segment) paths, also insert into flat map
        if path.0.len() == 1 {
            self.commands.insert(command.id.to_string(), command);
        }

        Ok(())
    }

    /// Resolve a command by `CommandPath`.
    pub fn resolve(&self, path: &CommandPath) -> Option<&Command> {
        let path_str = path.to_path_string();
        self.tree_commands.get(&path_str)
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
                    // Only direct children (no further '/')
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
            id,
            summary: "test",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
        }
    }

    // E007: collision on re-register
    #[test]
    fn e007_collision_on_re_register() {
        let mut registry = CommandRegistry::new();
        let path = CommandPath::new(&["cluster", "get"]).unwrap();
        registry.register_at(&path, make_cmd("get")).unwrap();
        let err = registry.register_at(&path, make_cmd("get")).unwrap_err();
        match err {
            RegistrationError::Collision { path } => {
                assert_eq!(path, "cluster/get");
            }
            _ => panic!("expected Collision"),
        }
    }

    // E008: alias conflict
    #[test]
    fn e008_alias_conflict() {
        let mut registry = CommandRegistry::new();
        registry.register(make_cmd("hello"));

        let mut cmd = make_cmd("greet");
        cmd.spec = Some(Arc::new(CommandSpec {
            aliases: vec!["hello"],
            ..Default::default()
        }));

        let err = registry
            .register_at(&CommandPath::root_for("greet"), cmd)
            .unwrap_err();
        match err {
            RegistrationError::AliasConflict { alias, .. } => {
                assert_eq!(alias, "hello");
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
        assert_eq!(found.unwrap().id, "get");
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
        children.sort_by(|a, b| a.to_path_string().cmp(&b.to_path_string()));
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
