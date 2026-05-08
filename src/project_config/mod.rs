//! # Project Config
//!
//! Utilities for discovering a project root directory by walking parent directories
//! and loading typed TOML configuration files.
//!
//! ## Precedence convention
//!
//! When callers layer multiple configuration sources, the recommended order (highest
//! to lowest precedence) is:
//!
//! 1. **CLI flags** — explicit arguments passed on the command line
//! 2. **Environment variables** — process environment at startup
//! 3. **Project TOML file** — discovered via [`find_file_upward`] and loaded via [`load_toml_file`]
//! 4. **Compiled defaults** — values baked into the binary at build time
//!
//! This module does not enforce or implement this layering; it is advisory only.
//! Callers own the merging logic.
//!
//! ## Quick start
//!
//! ```no_run
//! use cli_framework::project_config::find_and_load;
//!
//! #[derive(serde::Deserialize)]
//! struct MyConfig {
//!     pub setting: String,
//! }
//!
//! # fn main() -> Result<(), cli_framework::project_config::ProjectConfigError> {
//! let (cfg, root) = find_and_load::<MyConfig>(
//!     &std::env::current_dir().unwrap(),
//!     "my-app.toml",
//! )?;
//! println!("Config loaded from {:?}", root.root_dir);
//! # Ok(())
//! # }
//! ```

mod discover;
mod loader;

pub use discover::{find_file_upward, find_file_upward_with_options, DiscoverOptions, ProjectRoot};
pub use loader::{find_and_load, find_and_load_with_options, load_toml_file, load_toml_str};

#[derive(Debug, thiserror::Error)]
pub enum ProjectConfigError {
    #[error("PC001: config file '{file_name}' not found starting from '{start}'")]
    NotFound {
        file_name: String,
        start: std::path::PathBuf,
    },

    #[error("PC002: failed to parse config file '{path}': {source}")]
    ParseError {
        path: std::path::PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("PC003: failed to read config file '{path}': {source}")]
    IoError {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("PC004: max search depth {max_depth} exceeded starting from '{start}'")]
    DepthExceeded {
        max_depth: usize,
        start: std::path::PathBuf,
    },

    #[error("PC005: file_name '{file_name}' must not contain path separators")]
    InvalidFileName { file_name: String },
}
