//! Writable, versioned configuration storage: a byte-level [`ConfigBackend`]
//! (a file in the user profile, the Windows registry, or an in-memory test
//! double) beneath a typed [`ConfigStore<T>`] that owns serialization
//! (JSON by default, TOML selectable), atomic writes, schema-version
//! migrations, and a reload/subscription seam for long-running applications.
//!
//! Enable with `features = ["config"]`. Distinct from — and does not
//! replace — the [`crate::project_config`] module: that module's
//! upward-search TOML discovery remains the right tool for a developer tool
//! reading config out of the repo it's run inside; this module is for
//! settings that live in the user profile and that the application itself
//! writes back to (spec 016).
//!
//! Built on top of the above (spec 021, ADR 0072/0073): [`manifest`] is the
//! JSON document an application declares its configuration surface in;
//! [`Policy`] is the document an organisation authors for one profile;
//! [`resolution`] folds a manifest and the resolution-order layers
//! (`defaults -> recommended -> config file -> environment -> flags ->
//! builder overrides -> ENFORCED`) into resolved values plus provenance. All
//! three are plain data, available under plain `config`. The networked
//! fetchers (`PolicyClient`, `RoamingConfigClient`) live in [`managed`],
//! gated behind the `config-managed` feature.
//!
//! ```
//! use cli_framework::config::{ConfigStore, InMemoryBackend, VersionedConfig};
//! use serde::{Deserialize, Serialize};
//! use std::sync::Arc;
//!
//! #[derive(Default, Clone, Serialize, Deserialize)]
//! struct MyConfig {
//!     schema_version: u32,
//!     greeting: String,
//! }
//!
//! impl VersionedConfig for MyConfig {
//!     fn schema_version(&self) -> u32 {
//!         self.schema_version
//!     }
//!     fn set_schema_version(&mut self, version: u32) {
//!         self.schema_version = version;
//!     }
//! }
//!
//! # fn main() -> Result<(), cli_framework::config::ConfigError> {
//! let backend = Arc::new(InMemoryBackend::new());
//! let store = ConfigStore::<MyConfig>::new(backend, Default::default(), 1);
//!
//! // Empty backend -> defaults (spec 016 user story 5).
//! let cfg = store.load()?;
//! assert_eq!(cfg.greeting, "");
//!
//! // Round trip.
//! let mut cfg = cfg;
//! cfg.greeting = "hello".to_string();
//! store.save(&cfg)?;
//! assert_eq!(store.load()?.greeting, "hello");
//! # Ok(())
//! # }
//! ```

mod backend;
#[cfg(feature = "config-managed")]
pub(crate) mod commands;
mod error;
mod file_backend;
mod format;
mod handle;
mod in_memory_backend;
#[cfg(feature = "config-managed")]
pub mod managed;
pub mod manifest;
mod options;
mod policy;
#[cfg(windows)]
mod registry_backend;
pub mod resolution;
mod store;
mod versioned;

pub use backend::ConfigBackend;
pub use error::ConfigError;
pub use file_backend::FileBackend;
pub use format::ConfigFormat;
pub use handle::ConfigHandle;
pub use in_memory_backend::InMemoryBackend;
pub use options::ConfigOptions;
pub use policy::{Policy, StaleAction};
#[cfg(windows)]
pub use registry_backend::RegistryBackend;
pub use store::{ConfigStore, MigrationFn};
pub use versioned::VersionedConfig;
