//! The config service (spec 022): a mountable `axum` router that resolves a
//! caller's identity to an organisation-authored **Profile** and serves the
//! flattened [`crate::config::Policy`] for it, plus the application's
//! manifest and each user's roaming user-scoped document.
//!
//! Enable with `features = ["config-service"]` (implies `config` and
//! `api-server`; adds `sqlx-core` + `sqlx-postgres`, never the `sqlx`
//! facade — see the `Cargo.toml` comment on those dependencies).
//!
//! Module path: `crate::config::service`, a sibling of
//! `crate::config::managed` — `config-managed` is the *client* half of specs
//! 021/022 (fetches what this module serves); this is the *server* half.
//! Nested one level deeper than `config-managed`'s single `mod.rs` because
//! this slice is genuinely larger: storage traits, two backends each, an
//! auth seam, assignment/inheritance resolution, startup validation, and
//! the router itself all warrant their own files rather than one large one.
//!
//! # Quick start
//!
//! ```no_run
//! # use std::sync::Arc;
//! use cli_framework::api::ApiServerBuilder;
//! use cli_framework::config::service::{
//!     config_service_router, ConfigServiceState, FsPolicyStore, InMemoryUserConfigStore,
//! };
//! # use cli_framework::config::service::{CallerIdentity, ConfigServiceError};
//! # struct AllowAll;
//! # #[async_trait::async_trait]
//! # impl CallerIdentity for AllowAll {
//! #     async fn authenticate(&self, _h: Option<&str>) -> Result<serde_json::Value, ConfigServiceError> {
//! #         Ok(serde_json::json!({"sub": "svc"}))
//! #     }
//! # }
//!
//! # async fn run() -> anyhow::Result<()> {
//! let policy_store = Arc::new(FsPolicyStore::load("./config-bundle")?);
//! let user_config_store = Arc::new(InMemoryUserConfigStore::new());
//! let identity: Arc<dyn CallerIdentity> = Arc::new(AllowAll);
//!
//! let state = ConfigServiceState::new(policy_store, user_config_store, identity);
//! state.validate_at_startup().await?; // refuse to serve a broken policy set
//!
//! let server = ApiServerBuilder::new()
//!     .mount("/config", config_service_router(state))
//!     // .version(...) for your own API, as usual
//!     .build();
//! # let _ = server;
//! # Ok(())
//! # }
//! ```
//!
//! See `skill/examples/with_config_service/src/main.rs` for a complete,
//! runnable version — including the `cli-framework-oidc` adapter that
//! proves [`CallerIdentity`] actually composes with a real OIDC validator,
//! not just a stub.

mod assignment;
mod error;
mod fs_store;
mod identity;
mod inherit;
mod memory_store;
pub mod postgres;
mod router;
mod state;
mod store;
mod types;
mod validate;

pub use assignment::{resolve_profile, ResolvedAssignment};
pub use error::{
    ConfigServiceError, InheritanceError, PolicyValidationError, StartupValidationError,
    StoreError, UserConfigWriteError,
};
pub use fs_store::FsPolicyStore;
pub use identity::{CallerClaims, CallerIdentity};
pub use inherit::{combined_chain_version, flatten, resolve_chain};
pub use memory_store::InMemoryUserConfigStore;
pub use router::config_service_router;
pub use state::{
    ConfigServiceState, MatchedRule, PolicyLookupError, ResolveDiagnostic,
    DEFAULT_MAX_USER_CONFIG_BYTES,
};
pub use store::{PolicyStore, UserConfigStore};
pub use types::{AssignmentRule, RuleOperator, StoredManifest, StoredPolicy, StoredUserConfig};
pub use validate::{validate_all, validate_stored_policy};
