//! `SecretStore` capability: a small async trait for storing and retrieving
//! secrets, with pluggable backends.
//!
//! Enable with `features = ["secrets"]`. This pulls in only [`zeroize`] (a
//! tiny, MSRV-friendly crate) beyond what core `cli-framework` already
//! depends on unconditionally (`async-trait`). The heavier OpenBao/Vault
//! backend lives behind the separate `secrets-openbao` feature
//! ([`openbao`]), which adds no dependency beyond `reqwest` (already a core
//! dependency). An OS-native credential store backend (Windows Credential
//! Manager / macOS Keychain / Linux Secret Service) lives behind
//! `secrets-keychain` ([`keychain`]), which adds the `keyring` crate.
//!
//! ```
//! # #[tokio::main] async fn main() {
//! use cli_framework::secrets::{InMemorySecretStore, SecretKey, SecretStore, SecretValue};
//!
//! let store = InMemorySecretStore::new();
//! let key = SecretKey::new(["connection", "42", "refresh_token"]).unwrap();
//! store.put(&key, SecretValue::from("rt-abc123")).await.unwrap();
//! let value = store.get(&key).await.unwrap();
//! assert_eq!(value.expose_str().unwrap(), "rt-abc123");
//! # }
//! ```

mod env_file;
mod error;
mod in_memory;
mod key;
mod value;

#[cfg(feature = "secrets-openbao")]
pub mod openbao;

#[cfg(feature = "secrets-keychain")]
pub mod keychain;

pub use env_file::EnvFileSecretStore;
pub use error::SecretError;
pub use in_memory::InMemorySecretStore;
pub use key::{SecretKey, SecretKeyError};
pub use value::SecretValue;

use async_trait::async_trait;

/// Store and retrieve secrets by [`SecretKey`], independent of backend.
///
/// Implementations MUST be safe to hold behind an `Arc` and call
/// concurrently from multiple tasks (`Send + Sync`).
#[async_trait]
pub trait SecretStore: Send + Sync {
    /// Fetch the current value for `key`. Returns [`SecretError::NotFound`]
    /// if nothing is stored under it.
    async fn get(&self, key: &SecretKey) -> Result<SecretValue, SecretError>;

    /// Store `value` under `key`, overwriting any existing value. Overwrite
    /// (rather than requiring a separate update op) is deliberate: it's what
    /// lets provider-side OAuth refresh-token rotation be a single call.
    async fn put(&self, key: &SecretKey, value: SecretValue) -> Result<(), SecretError>;

    /// Remove the value stored under `key`. Backends treat deleting an
    /// already-absent key as a no-op success, not an error.
    async fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;

    /// Ask the backend to mint and return a fresh value for `key` (e.g. a
    /// generated signing key). Backends that cannot generate material
    /// return [`SecretError::NotSupported`] — this is expected for R1 on
    /// every backend shipped here; `rotate` is a forward-looking seam.
    async fn rotate(&self, key: &SecretKey) -> Result<SecretValue, SecretError>;
}
