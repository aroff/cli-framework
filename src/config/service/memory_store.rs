//! [`InMemoryUserConfigStore`]: the test/dev [`UserConfigStore`] backend
//! (spec 022 user story 30 — "run and test the service without Postgres").

use super::error::{StoreError, UserConfigWriteError};
use super::store::UserConfigStore;
use super::types::StoredUserConfig;
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct InMemoryUserConfigStore {
    docs: Mutex<HashMap<(String, String), StoredUserConfig>>,
}

impl InMemoryUserConfigStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl UserConfigStore for InMemoryUserConfigStore {
    async fn get(&self, app: &str, subject: &str) -> Result<StoredUserConfig, StoreError> {
        let docs = self
            .docs
            .lock()
            .map_err(|_| StoreError::backend("in-memory user config store lock poisoned"))?;
        Ok(docs
            .get(&(app.to_string(), subject.to_string()))
            .cloned()
            .unwrap_or_else(|| StoredUserConfig {
                app: app.to_string(),
                subject: subject.to_string(),
                doc: Map::new(),
                version: 0,
            }))
    }

    async fn put(
        &self,
        app: &str,
        subject: &str,
        doc: Map<String, Value>,
        expected_version: u64,
    ) -> Result<u64, UserConfigWriteError> {
        let mut docs = self
            .docs
            .lock()
            .map_err(|_| StoreError::backend("in-memory user config store lock poisoned"))?;
        let key = (app.to_string(), subject.to_string());
        let current_version = docs.get(&key).map(|d| d.version).unwrap_or(0);
        if current_version != expected_version {
            return Err(UserConfigWriteError::Conflict {
                current: current_version,
                expected: expected_version,
            });
        }
        let new_version = current_version + 1;
        docs.insert(
            key,
            StoredUserConfig {
                app: app.to_string(),
                subject: subject.to_string(),
                doc,
                version: new_version,
            },
        );
        Ok(new_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn get_on_a_never_written_subject_is_version_zero_and_empty() {
        let store = InMemoryUserConfigStore::new();
        let doc = store.get("app", "u1").await.unwrap();
        assert_eq!(doc.version, 0);
        assert!(doc.doc.is_empty());
    }

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let store = InMemoryUserConfigStore::new();
        let mut doc = Map::new();
        doc.insert("nickname".to_string(), json!("alice"));
        let version = store.put("app", "u1", doc.clone(), 0).await.unwrap();
        assert_eq!(version, 1);

        let got = store.get("app", "u1").await.unwrap();
        assert_eq!(got.doc, doc);
        assert_eq!(got.version, 1);
    }

    #[tokio::test]
    async fn stale_expected_version_is_rejected_and_leaves_document_unchanged() {
        let store = InMemoryUserConfigStore::new();
        let mut doc = Map::new();
        doc.insert("k".to_string(), json!("v1"));
        store.put("app", "u1", doc, 0).await.unwrap();

        let mut conflicting = Map::new();
        conflicting.insert("k".to_string(), json!("v2-should-not-land"));
        let err = store.put("app", "u1", conflicting, 0).await.unwrap_err();
        assert!(matches!(
            err,
            UserConfigWriteError::Conflict {
                current: 1,
                expected: 0
            }
        ));

        let got = store.get("app", "u1").await.unwrap();
        assert_eq!(got.doc.get("k"), Some(&json!("v1")));
        assert_eq!(got.version, 1);
    }

    #[tokio::test]
    async fn two_subjects_are_isolated() {
        let store = InMemoryUserConfigStore::new();
        let mut doc = Map::new();
        doc.insert("k".to_string(), json!("alice's value"));
        store.put("app", "alice", doc, 0).await.unwrap();

        let bob_doc = store.get("app", "bob").await.unwrap();
        assert!(bob_doc.doc.is_empty(), "bob must not see alice's document");
    }

    #[tokio::test]
    async fn two_apps_for_the_same_subject_are_isolated() {
        let store = InMemoryUserConfigStore::new();
        let mut doc = Map::new();
        doc.insert("k".to_string(), json!("app-a-value"));
        store.put("app-a", "u1", doc, 0).await.unwrap();

        let other = store.get("app-b", "u1").await.unwrap();
        assert!(other.doc.is_empty());
    }
}
