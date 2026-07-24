//! Trait-conformance suite for `SecretStore`: the same observable contract
//! must hold across every backend. Runs against `InMemorySecretStore` and
//! `EnvFileSecretStore` unconditionally (mandatory, no external deps). See
//! `tests/unit/secrets_openbao_conformance.rs` (behind `secrets-openbao`)
//! for the OpenBao backend, opt-in via env var since this sandbox has no
//! network path to pull a Vault/OpenBao image.

use cli_framework::secrets::{
    EnvFileSecretStore, InMemorySecretStore, SecretError, SecretKey, SecretStore, SecretValue,
};
use tempfile::TempDir;

async fn assert_conformance(store: &dyn SecretStore) {
    let key = SecretKey::new(["conformance", "widget"]).unwrap();

    // get-missing → NotFound
    let err = store.get(&key).await.unwrap_err();
    assert!(
        matches!(err, SecretError::NotFound),
        "expected NotFound for a missing key, got {err:?}"
    );

    // put → get round-trip
    store
        .put(&key, SecretValue::from("first-value"))
        .await
        .expect("put should succeed");
    let got = store.get(&key).await.expect("get after put");
    assert_eq!(got.expose_str().unwrap(), "first-value");

    // overwrite via put
    store
        .put(&key, SecretValue::from("second-value"))
        .await
        .expect("overwrite put should succeed");
    let got = store.get(&key).await.expect("get after overwrite");
    assert_eq!(got.expose_str().unwrap(), "second-value");

    // delete → subsequent get NotFound
    store.delete(&key).await.expect("delete should succeed");
    let err = store.get(&key).await.unwrap_err();
    assert!(
        matches!(err, SecretError::NotFound),
        "expected NotFound after delete, got {err:?}"
    );

    // delete of an already-absent key is idempotent, not an error
    store
        .delete(&key)
        .await
        .expect("deleting an absent key should be a no-op success");
}

#[tokio::test]
async fn in_memory_store_is_conformant() {
    let store = InMemorySecretStore::new();
    assert_conformance(&store).await;
}

#[tokio::test]
async fn env_file_store_is_conformant() {
    let dir = TempDir::new().unwrap();
    let store = EnvFileSecretStore::new(dir.path());
    assert_conformance(&store).await;
}

// ── rotate: NotSupported on both light-feature backends ─────────────────────

#[tokio::test]
async fn in_memory_rotate_is_not_supported() {
    let store = InMemorySecretStore::new();
    let key = SecretKey::new(["rotate", "target"]).unwrap();
    let err = store.rotate(&key).await.unwrap_err();
    assert!(matches!(err, SecretError::NotSupported(_)));
}

#[tokio::test]
async fn env_file_rotate_is_not_supported() {
    let dir = TempDir::new().unwrap();
    let store = EnvFileSecretStore::new(dir.path());
    let key = SecretKey::new(["rotate", "target"]).unwrap();
    let err = store.rotate(&key).await.unwrap_err();
    assert!(matches!(err, SecretError::NotSupported(_)));
}

// ── env override behavior specific to EnvFileSecretStore ────────────────────

#[tokio::test]
async fn env_file_store_isolates_different_base_dirs() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    let store_a = EnvFileSecretStore::new(dir_a.path());
    let store_b = EnvFileSecretStore::new(dir_b.path());
    let key = SecretKey::new(["shared", "key"]).unwrap();

    store_a.put(&key, SecretValue::from("a")).await.unwrap();
    assert!(matches!(
        store_b.get(&key).await.unwrap_err(),
        SecretError::NotFound
    ));
    let v = store_a.get(&key).await.unwrap();
    assert_eq!(v.expose_str().unwrap(), "a");
}
