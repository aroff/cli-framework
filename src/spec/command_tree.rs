use crate::spec::arg_spec::ArgSpec;

/// Command-level metadata for typed spec-driven commands.
#[derive(Debug, Clone, Default)]
pub struct CommandSpec {
    pub summary: &'static str,
    pub long_about: Option<&'static str>,
    pub examples: Vec<&'static str>,
    pub aliases: Vec<&'static str>,
    pub hidden: bool,
    /// Deprecation message, if this command is deprecated.
    pub deprecated: Option<&'static str>,
    pub env_vars: Vec<EnvVarEntry>,
    pub exit_codes: Vec<ExitCodeEntry>,
    pub args: Vec<ArgSpec>,
    pub notes: Option<&'static str>,
}

/// An environment variable referenced by a command.
#[derive(Debug, Clone)]
pub struct EnvVarEntry {
    pub name: &'static str,
    pub description: &'static str,
}

/// An exit code documented by a command.
#[derive(Debug, Clone)]
pub struct ExitCodeEntry {
    pub code: i32,
    pub description: &'static str,
}

/// Metadata for a command group (non-leaf path node).
#[derive(Debug, Clone, Default)]
pub struct GroupMetadata {
    pub summary: &'static str,
    pub hidden: bool,
}

/// Hierarchical command path (e.g. `["cluster", "get"]` → `"cluster/get"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CommandPath(pub Vec<String>);

impl CommandPath {
    /// Construct from string slices. Returns Err if any segment contains '/'.
    pub fn new(segments: &[&str]) -> Result<Self, CommandPathError> {
        for s in segments {
            if s.contains('/') {
                return Err(CommandPathError::InvalidSegment {
                    segment: s.to_string(),
                });
            }
        }
        Ok(CommandPath(
            segments.iter().map(|s| s.to_string()).collect(),
        ))
    }

    /// Convenience for a single root-level ID.
    pub fn root_for(id: &str) -> Self {
        CommandPath(vec![id.to_string()])
    }

    /// Returns `"a/b/c"` for path `["a", "b", "c"]`.
    pub fn to_path_string(&self) -> String {
        self.0.join("/")
    }

    /// Returns `None` for a root-level (single-segment) path.
    pub fn parent(&self) -> Option<CommandPath> {
        if self.0.len() <= 1 {
            None
        } else {
            Some(CommandPath(self.0[..self.0.len() - 1].to_vec()))
        }
    }

    /// Returns a new path with the given segment appended.
    pub fn push(&self, segment: &str) -> Result<CommandPath, CommandPathError> {
        if segment.contains('/') {
            return Err(CommandPathError::InvalidSegment {
                segment: segment.to_string(),
            });
        }
        let mut new_path = self.0.clone();
        new_path.push(segment.to_string());
        Ok(CommandPath(new_path))
    }

    /// Returns the final segment (leaf command name).
    pub fn leaf(&self) -> Option<&str> {
        self.0.last().map(|s| s.as_str())
    }
}

/// Error constructing a CommandPath.
#[derive(Debug, thiserror::Error)]
pub enum CommandPathError {
    #[error("path segment '{segment}' must not contain '/'")]
    InvalidSegment { segment: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_to_path_string_roundtrip() {
        let path = CommandPath::new(&["cluster", "get"]).unwrap();
        assert_eq!(path.to_path_string(), "cluster/get");
    }

    #[test]
    fn path_single_segment_to_string() {
        let path = CommandPath::root_for("hello");
        assert_eq!(path.to_path_string(), "hello");
    }

    #[test]
    fn path_parent_root_is_none() {
        let path = CommandPath::root_for("hello");
        assert!(path.parent().is_none());
    }

    #[test]
    fn path_parent_nested() {
        let path = CommandPath::new(&["cluster", "get"]).unwrap();
        assert_eq!(path.parent(), Some(CommandPath::root_for("cluster")));
    }

    #[test]
    fn path_push_success() {
        let path = CommandPath::root_for("cluster");
        let pushed = path.push("get").unwrap();
        assert_eq!(pushed.to_path_string(), "cluster/get");
    }

    #[test]
    fn path_push_slash_segment_error() {
        let path = CommandPath::root_for("cluster");
        let err = path.push("bad/segment").unwrap_err();
        match err {
            CommandPathError::InvalidSegment { segment } => {
                assert_eq!(segment, "bad/segment");
            }
        }
    }

    #[test]
    fn path_new_invalid_segment_error() {
        let err = CommandPath::new(&["bad/segment"]).unwrap_err();
        match err {
            CommandPathError::InvalidSegment { segment } => {
                assert_eq!(segment, "bad/segment");
            }
        }
    }

    #[test]
    fn path_leaf() {
        let path = CommandPath::new(&["cluster", "get"]).unwrap();
        assert_eq!(path.leaf(), Some("get"));
    }

    #[test]
    fn path_leaf_root() {
        let path = CommandPath::root_for("hello");
        assert_eq!(path.leaf(), Some("hello"));
    }

    #[test]
    fn path_empty_leaf_is_none() {
        let path = CommandPath(vec![]);
        assert!(path.leaf().is_none());
    }
}
