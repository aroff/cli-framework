//! [`InMemorySecretStore`]: a process-local backend for tests and local dev.

use super::{SecretError, SecretKey, SecretStore, SecretValue};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

/// A `SecretStore` backed by a process-local `HashMap`. Nothing persists
/// across process restarts; intended for unit tests and quick local dev
/// where even the [`super::EnvFileSecretStore`] on-disk footprint is
/// unwanted.
///
/// `rotate` returns [`SecretError::NotSupported`] — there's no
/// backend-generated material to mint here.
#[derive(Default)]
pub struct InMemorySecretStore {
    entries: RwLock<HashMap<String, SecretValue>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SecretStore for InMemorySecretStore {
    async fn get(&self, key: &SecretKey) -> Result<SecretValue, SecretError> {
        self.entries
            .read()
            .expect("InMemorySecretStore lock poisoned")
            .get(key.as_str())
            .cloned()
            .ok_or(SecretError::NotFound)
    }

    async fn put(&self, key: &SecretKey, value: SecretValue) -> Result<(), SecretError> {
        self.entries
            .write()
            .expect("InMemorySecretStore lock poisoned")
            .insert(key.as_str().to_string(), value);
        Ok(())
    }

    async fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        // Idempotent: deleting a key that isn't present is not an error,
        // matching the file/OpenBao backends.
        self.entries
            .write()
            .expect("InMemorySecretStore lock poisoned")
            .remove(key.as_str());
        Ok(())
    }

    async fn rotate(&self, _key: &SecretKey) -> Result<SecretValue, SecretError> {
        Err(SecretError::NotSupported(
            "rotate is not supported by InMemorySecretStore",
        ))
    }
}
