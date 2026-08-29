//! Live-OS-keychain trait-conformance test — opt-in only.
//!
//! `KeychainSecretStore` talks to a real OS credential store: on Linux
//! that's the Secret Service over D-Bus, which needs a running D-Bus
//! session bus AND an active, unlocked provider (`gnome-keyring-daemon`,
//! KWallet, ...) registered as `org.freedesktop.secrets`. This sandbox has
//! a session bus but no such provider running (confirmed with
//! `dbus-send --session --dest=org.freedesktop.secrets ... Peer.Ping` →
//! `ServiceUnknown`), so this test can't be exercised here — the same kind
//! of environment gap `secrets_openbao_conformance.rs` documents for a
//! Docker-Hub-less sandbox.
//!
//! Gated behind `CFW_TEST_KEYCHAIN_LIVE=1` and SKIPS (does not fail) when
//! that's unset, mirroring the OpenBao precedent: "gate ONLY the live test
//! behind an env flag ... in-memory + env/file conformance are MANDATORY"
//! (PRD-005). The mapping-only coverage (no OS access required — the
//! `(service, username)` identity mapping produces distinct entries for
//! distinct `SecretKey`s) lives as pure unit tests inside
//! `src/secrets/keychain.rs` itself, and always runs.
//!
//! Run this on a machine with a real, unlocked credential store:
//!
//! ```sh
//! CFW_TEST_KEYCHAIN_LIVE=1 cargo test --features secrets-keychain \
//!     --test unit_secrets_keychain_conformance
//! ```
//!
//! On Linux, that means a logged-in desktop session (or `dbus-run-session
//! gnome-keyring-daemon --unlock` with a keyring already provisioned) — a
//! bare `dbus-launch` alone is not enough, since nothing then owns
//! `org.freedesktop.secrets` on that bus.

use cli_framework::secrets::keychain::KeychainSecretStore;
use cli_framework::secrets::{SecretError, SecretKey, SecretStore, SecretValue};

#[tokio::test]
async fn keychain_backend_is_conformant() {
    if std::env::var("CFW_TEST_KEYCHAIN_LIVE").is_err() {
        eprintln!(
            "skipping keychain_backend_is_conformant: set CFW_TEST_KEYCHAIN_LIVE=1 \
             to run against a real, unlocked OS credential store (a headless \
             sandbox typically has neither a running Secret Service provider \
             on Linux nor an interactive macOS/Windows session)"
        );
        return;
    }

    // A service prefix scoped to this test run so it never collides with a
    // real application's entries in whatever credential store is live.
    let store = KeychainSecretStore::new("cli-framework-conformance-test");
    let key = SecretKey::new(["conformance", "keychain-widget"]).unwrap();

    // Best-effort cleanup up front in case a previous run left this entry
    // behind (e.g. the test was interrupted between put and delete).
    let _ = store.delete(&key).await;

    // get-missing → NotFound
    let err = store.get(&key).await.unwrap_err();
    assert!(matches!(err, SecretError::NotFound), "got {err:?}");

    // put → get round-trip
    store
        .put(&key, SecretValue::from("first-value"))
        .await
        .expect("put");
    let got = store.get(&key).await.expect("get after put");
    assert_eq!(got.expose_str().unwrap(), "first-value");

    // overwrite via put
    store
        .put(&key, SecretValue::from("second-value"))
        .await
        .expect("overwrite put");
    let got = store.get(&key).await.expect("get after overwrite");
    assert_eq!(got.expose_str().unwrap(), "second-value");

    // delete → subsequent get NotFound
    store.delete(&key).await.expect("delete");
    let err = store.get(&key).await.unwrap_err();
    assert!(matches!(err, SecretError::NotFound), "got {err:?}");

    // delete of an already-absent key is idempotent, not an error
    store
        .delete(&key)
        .await
        .expect("deleting an absent key should be a no-op success");

    // rotate is NotSupported — no backend-generated material to mint
    let err = store.rotate(&key).await.unwrap_err();
    assert!(matches!(err, SecretError::NotSupported(_)), "got {err:?}");
}

#[tokio::test]
async fn keychain_distinct_services_do_not_see_each_others_entries() {
    if std::env::var("CFW_TEST_KEYCHAIN_LIVE").is_err() {
        eprintln!(
            "skipping keychain_distinct_services_do_not_see_each_others_entries: \
             set CFW_TEST_KEYCHAIN_LIVE=1 (see keychain_backend_is_conformant)"
        );
        return;
    }

    let store_a = KeychainSecretStore::new("cli-framework-conformance-test-a");
    let store_b = KeychainSecretStore::new("cli-framework-conformance-test-b");
    let key = SecretKey::new(["conformance", "shared-key"]).unwrap();

    let _ = store_a.delete(&key).await;
    let _ = store_b.delete(&key).await;

    store_a
        .put(&key, SecretValue::from("a-value"))
        .await
        .expect("put a");
    assert!(
        matches!(store_b.get(&key).await.unwrap_err(), SecretError::NotFound),
        "a different service prefix must not see store_a's entry"
    );
    let got = store_a.get(&key).await.expect("get after put");
    assert_eq!(got.expose_str().unwrap(), "a-value");

    store_a.delete(&key).await.expect("cleanup a");
}
