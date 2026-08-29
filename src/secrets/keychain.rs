//! [`KeychainSecretStore`]: a `SecretStore` backed by the OS-native
//! credential store.
//!
//! Behind the `secrets-keychain` feature. Wraps the [`keyring`] crate
//! (v4 — its `v1`-feature compatibility module, the simple,
//! platform-uniform `Entry` API that survived keyring's 4.0 rewrite onto
//! `keyring-core`) so this crate never talks to D-Bus / Keychain Services /
//! Credential Manager directly:
//!
//! - macOS: Keychain Services.
//! - Windows: Windows Credential Manager.
//! - Linux/BSD: the Secret Service, over D-Bus, via keyring's default `v1`
//!   feature pulling in `zbus-secret-service-keyring-store` (a pure-Rust
//!   D-Bus client — no `libdbus-dev`/`pkg-config` needed at build time).
//!
//! `keyring = "4.1"` is added as a plain optional dependency with default
//! features (the `v1` feature is on by default and is exactly the surface
//! used here); no extra feature selection is needed to get all three native
//! backends. Note `keyring` 4.x's own declared `rust-version` (1.88.0) is
//! newer than this workspace's baseline (1.83.0) — that only bites a
//! consumer who enables `secrets-keychain` specifically, same as any other
//! optional dependency that raises the *effective* MSRV for the feature
//! that pulls it in.
//!
//! ## Identity mapping
//!
//! `keyring` identifies every credential by an OS-level `(service,
//! username)` pair. [`KeychainSecretStore`] maps a [`SecretKey`] onto that
//! pair as:
//!
//! - `service`: a caller-supplied prefix, fixed for the life of the store
//!   (see [`KeychainSecretStore::new`]). This is what keeps two different
//!   applications sharing the same machine (and possibly the same
//!   [`SecretKey`] namespace — nothing stops two apps from both using
//!   `"oauth/client_secret"`) from reading or overwriting each other's
//!   credentials: give each application its own `service` value (e.g. its
//!   bundle ID or crate name).
//! - `username` (the account field): [`SecretKey::as_str`] verbatim — the
//!   full `/`-joined path, e.g. `"connection/42/refresh_token"`. Two
//!   different keys under the same `service` therefore always resolve to
//!   different credentials. This also means OS-native tooling (macOS
//!   Keychain Access, Windows Credential Manager, `secret-tool`/Seahorse on
//!   Linux) shows the raw key path as the account name when a human
//!   browses the store by hand — deliberate, for identifiability.
//!
//! The mapping itself is [`keyring_identity`], factored out so it can be
//! unit-tested without touching a real OS credential store (see the tests
//! below); `tests/unit/secrets_keychain_conformance.rs` covers the
//! live-backend round trip, gated behind an opt-in env var since a headless
//! CI sandbox typically has no running D-Bus session / unlocked keyring.
//!
//! ## Blocking calls
//!
//! `keyring::Entry`'s methods are synchronous, blocking OS/D-Bus calls.
//! Every method here off-loads the call to [`tokio::task::spawn_blocking`]
//! — the same convention [`super::EnvFileSecretStore`] uses for its
//! blocking filesystem I/O — so a `SecretStore::get`/`put`/`delete` call
//! never stalls an async runtime's worker threads.
//!
//! ## Scope
//!
//! - `rotate` returns [`SecretError::NotSupported`], exactly like
//!   [`super::InMemorySecretStore`] — there's no backend-generated material
//!   to mint here either.
//! - Values must be valid UTF-8: `put`/`get` go through keyring's
//!   string-oriented `set_password`/`get_password` (not the byte-oriented
//!   `set_secret`/`get_secret`), matching [`super::OpenBaoSecretStore`]'s
//!   R1 UTF-8-only scope. A non-UTF-8 `put` fails with
//!   [`SecretError::Backend`].

use super::{SecretError, SecretKey, SecretStore, SecretValue};
use async_trait::async_trait;

/// `SecretStore` backed by the OS-native credential store via the
/// [`keyring`] crate: Windows Credential Manager, macOS Keychain, or (on
/// Linux/BSD) the Secret Service over D-Bus.
///
/// See the module docs for the `(service, username)` identity mapping.
pub struct KeychainSecretStore {
    service: String,
}

impl KeychainSecretStore {
    /// `service` namespaces every credential this store touches — pass
    /// something stable and specific to your application (e.g. its bundle
    /// ID or crate name, like `"com.example.aidesktop"`), not a generic
    /// value, so it can't collide with another application's entries in
    /// the same OS credential store. See the module docs for the full
    /// `(service, username)` mapping.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

/// The `(service, username)` pair a given [`SecretKey`] resolves to under
/// `service`. Factored out of the trait methods so the mapping (no
/// collisions between distinct keys, or between distinct services sharing
/// a key) can be verified without touching a real OS credential store.
fn keyring_identity(service: &str, key: &SecretKey) -> (String, String) {
    (service.to_string(), key.as_str().to_string())
}

/// Map a [`keyring::Error`] onto this crate's [`SecretError`].
///
/// `keyring::Error` is `#[non_exhaustive]` upstream, so this match ends in
/// a catch-all — anything not called out explicitly becomes
/// [`SecretError::Backend`], which preserves the original error via
/// `#[source]` rather than discarding it.
fn map_keyring_err(e: keyring::Error) -> SecretError {
    match &e {
        keyring::Error::NoEntry => SecretError::NotFound,
        // The store exists but couldn't be reached right now (typically:
        // locked). That's exactly the "retryable, backend-side" shape
        // `Unavailable` documents — unlike `PermissionDenied`, which this
        // crate reserves for the backend understanding and refusing a
        // request (auth/ACL), not merely being temporarily inaccessible.
        keyring::Error::NoStorageAccess(_) => SecretError::Unavailable(format!(
            "keychain credential store is inaccessible (locked?): {e}"
        )),
        _ => SecretError::backend(e),
    }
}

#[async_trait]
impl SecretStore for KeychainSecretStore {
    async fn get(&self, key: &SecretKey) -> Result<SecretValue, SecretError> {
        let (service, account) = keyring_identity(&self.service, key);
        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(&service, &account).map_err(map_keyring_err)?;
            entry
                .get_password()
                .map(SecretValue::from)
                .map_err(map_keyring_err)
        })
        .await
        .map_err(SecretError::backend)?
    }

    async fn put(&self, key: &SecretKey, value: SecretValue) -> Result<(), SecretError> {
        let (service, account) = keyring_identity(&self.service, key);
        let text = value
            .expose_str()
            .map_err(|e| {
                SecretError::backend(format!(
                    "KeychainSecretStore requires UTF-8 secret values: {e}"
                ))
            })?
            .to_string();
        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(&service, &account).map_err(map_keyring_err)?;
            entry.set_password(&text).map_err(map_keyring_err)
        })
        .await
        .map_err(SecretError::backend)?
    }

    async fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let (service, account) = keyring_identity(&self.service, key);
        tokio::task::spawn_blocking(move || {
            let entry = keyring::Entry::new(&service, &account).map_err(map_keyring_err)?;
            match entry.delete_credential() {
                Ok(()) => Ok(()),
                // Idempotent, matching the trait's documented contract
                // (see `SecretStore::delete`) and the in-memory/env-file
                // backends' own handling of an already-absent key.
                Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(map_keyring_err(e)),
            }
        })
        .await
        .map_err(SecretError::backend)?
    }

    async fn rotate(&self, _key: &SecretKey) -> Result<SecretValue, SecretError> {
        Err(SecretError::NotSupported(
            "rotate is not supported by KeychainSecretStore",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_keys_under_the_same_service_do_not_collide() {
        let a = keyring_identity("svc", &SecretKey::parse("a/b").unwrap());
        let b = keyring_identity("svc", &SecretKey::parse("a/c").unwrap());
        assert_ne!(a, b, "different keys must map to different identities");
        assert_eq!(a.0, b.0, "same service prefix should be preserved");
        assert_eq!(a.1, "a/b");
        assert_eq!(b.1, "a/c");
    }

    #[test]
    fn distinct_services_isolate_the_same_key() {
        let key = SecretKey::parse("shared/key").unwrap();
        let a = keyring_identity("app-one", &key);
        let b = keyring_identity("app-two", &key);
        assert_ne!(
            a, b,
            "the same key under different service prefixes must not collide"
        );
        assert_eq!(a.1, b.1, "the account/username half is the key path");
    }

    #[test]
    fn account_is_the_full_slash_joined_key_path() {
        let key = SecretKey::new(["connection", "42", "refresh_token"]).unwrap();
        let (service, account) = keyring_identity("my-app", &key);
        assert_eq!(service, "my-app");
        assert_eq!(account, "connection/42/refresh_token");
    }
}
