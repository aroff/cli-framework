//! [`OpenBaoSecretStore`]: a `SecretStore` backed by OpenBao/Vault KV v2.
//!
//! Behind the `secrets-openbao` feature. Talks plain REST over `reqwest`
//! (already a core `cli-framework` dependency) — no vault client crate.
//!
//! R1 scope (see PRD-005 / ADR-0004 in the corpus repo):
//! - Values must be valid UTF-8 (OAuth client secrets, refresh tokens, and
//!   signing keys all are). Non-UTF-8 `put` fails with
//!   [`SecretError::Backend`]. Lifting this later (e.g. base64-encoding raw
//!   bytes) is additive and doesn't change the trait.
//! - `rotate` always returns [`SecretError::NotSupported`] — OpenBao secret
//!   generation isn't wired up in R1.
//! - AppRole tokens are fetched once and cached for the store's lifetime;
//!   there is no automatic re-login-and-retry on a `403` from an expired
//!   token. A caller that hits [`SecretError::PermissionDenied`] under
//!   AppRole auth should reconstruct the store.

use super::{SecretError, SecretKey, SecretStore, SecretValue};
use async_trait::async_trait;
use std::fmt;

/// OpenBao/Vault connection + auth configuration.
#[derive(Clone)]
pub struct OpenBaoConfig {
    /// Base address, e.g. `https://vault.example.com:8200` (trailing slash
    /// optional).
    pub address: String,
    pub auth: OpenBaoAuth,
    /// KV v2 mount point, e.g. `"secret"`.
    pub mount: String,
    /// Optional Vault Enterprise / OpenBao namespace.
    pub namespace: Option<String>,
}

impl fmt::Debug for OpenBaoConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenBaoConfig")
            .field("address", &self.address)
            .field("auth", &self.auth)
            .field("mount", &self.mount)
            .field("namespace", &self.namespace)
            .finish()
    }
}

/// How the store authenticates to OpenBao/Vault.
#[derive(Clone)]
pub enum OpenBaoAuth {
    /// A pre-issued token, used as-is.
    Token(String),
    /// AppRole credentials, exchanged for a client token on first use.
    AppRole { role_id: String, secret_id: String },
}

/// Redacts token/secret_id material — this type is otherwise easy to end up
/// in a `Debug`-derived parent struct (like [`OpenBaoConfig`]) or a log line.
impl fmt::Debug for OpenBaoAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenBaoAuth::Token(_) => f.debug_tuple("Token").field(&"[redacted]").finish(),
            OpenBaoAuth::AppRole { role_id, .. } => f
                .debug_struct("AppRole")
                .field("role_id", role_id)
                .field("secret_id", &"[redacted]")
                .finish(),
        }
    }
}

/// `SecretStore` backed by an OpenBao/Vault KV v2 mount.
pub struct OpenBaoSecretStore {
    http: reqwest::Client,
    address: String,
    mount: String,
    namespace: Option<String>,
    auth: OpenBaoAuth,
    token: tokio::sync::RwLock<Option<String>>,
}

impl OpenBaoSecretStore {
    pub fn new(config: OpenBaoConfig) -> Self {
        let address = config.address.trim_end_matches('/').to_string();
        let initial_token = match &config.auth {
            OpenBaoAuth::Token(t) => Some(t.clone()),
            OpenBaoAuth::AppRole { .. } => None,
        };
        Self {
            http: reqwest::Client::new(),
            address,
            mount: config.mount,
            namespace: config.namespace,
            auth: config.auth,
            token: tokio::sync::RwLock::new(initial_token),
        }
    }

    fn data_url(&self, key: &SecretKey) -> String {
        format!("{}/v1/{}/data/{}", self.address, self.mount, key.as_str())
    }

    async fn current_token(&self) -> Result<String, SecretError> {
        if let Some(t) = self.token.read().await.clone() {
            return Ok(t);
        }
        self.login().await
    }

    async fn login(&self) -> Result<String, SecretError> {
        match &self.auth {
            OpenBaoAuth::Token(t) => {
                *self.token.write().await = Some(t.clone());
                Ok(t.clone())
            }
            OpenBaoAuth::AppRole { role_id, secret_id } => {
                let url = format!("{}/v1/auth/approle/login", self.address);
                let mut req = self.http.post(&url);
                if let Some(ns) = &self.namespace {
                    req = req.header("X-Vault-Namespace", ns.as_str());
                }
                let resp = req
                    .json(&serde_json::json!({ "role_id": role_id, "secret_id": secret_id }))
                    .send()
                    .await
                    .map_err(map_reqwest_err)?;
                if !resp.status().is_success() {
                    return Err(Self::status_error(resp.status(), "approle login"));
                }
                let body: serde_json::Value = resp.json().await.map_err(SecretError::backend)?;
                let token = body["auth"]["client_token"]
                    .as_str()
                    .ok_or_else(|| {
                        SecretError::backend("approle login response missing auth.client_token")
                    })?
                    .to_string();
                *self.token.write().await = Some(token.clone());
                Ok(token)
            }
        }
    }

    fn status_error(status: reqwest::StatusCode, context: &str) -> SecretError {
        match status.as_u16() {
            404 => SecretError::NotFound,
            403 => SecretError::PermissionDenied,
            429 | 500..=599 => SecretError::Unavailable(format!("{context}: HTTP {status}")),
            _ => SecretError::backend(format!("{context}: unexpected HTTP {status}")),
        }
    }

    fn authed(&self, builder: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
        let mut b = builder.header("X-Vault-Token", token);
        if let Some(ns) = &self.namespace {
            b = b.header("X-Vault-Namespace", ns.as_str());
        }
        b
    }
}

fn map_reqwest_err(e: reqwest::Error) -> SecretError {
    if e.is_connect() || e.is_timeout() {
        SecretError::Unavailable(e.to_string())
    } else {
        SecretError::backend(e)
    }
}

#[async_trait]
impl SecretStore for OpenBaoSecretStore {
    async fn get(&self, key: &SecretKey) -> Result<SecretValue, SecretError> {
        let token = self.current_token().await?;
        let url = self.data_url(key);
        let resp = self
            .authed(self.http.get(&url), &token)
            .send()
            .await
            .map_err(map_reqwest_err)?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SecretError::NotFound);
        }
        if !resp.status().is_success() {
            return Err(Self::status_error(resp.status(), "openbao get"));
        }

        let body: serde_json::Value = resp.json().await.map_err(SecretError::backend)?;
        let inner = &body["data"]["data"];
        // KV v2 returns 200 with data.data == null for a soft-deleted version.
        if inner.is_null() {
            return Err(SecretError::NotFound);
        }
        let value = inner["value"].as_str().ok_or_else(|| {
            SecretError::backend("openbao KV v2 response missing data.data.value")
        })?;
        Ok(SecretValue::from(value.to_string()))
    }

    async fn put(&self, key: &SecretKey, value: SecretValue) -> Result<(), SecretError> {
        let token = self.current_token().await?;
        let text = value.expose_str().map_err(|e| {
            SecretError::backend(format!(
                "OpenBaoSecretStore requires UTF-8 secret values in R1: {e}"
            ))
        })?;
        let url = self.data_url(key);
        let resp = self
            .authed(self.http.post(&url), &token)
            .json(&serde_json::json!({ "data": { "value": text } }))
            .send()
            .await
            .map_err(map_reqwest_err)?;

        if !resp.status().is_success() {
            return Err(Self::status_error(resp.status(), "openbao put"));
        }
        Ok(())
    }

    async fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let token = self.current_token().await?;
        let url = self.data_url(key);
        let resp = self
            .authed(self.http.delete(&url), &token)
            .send()
            .await
            .map_err(map_reqwest_err)?;

        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(Self::status_error(resp.status(), "openbao delete"))
    }

    async fn rotate(&self, _key: &SecretKey) -> Result<SecretValue, SecretError> {
        Err(SecretError::NotSupported(
            "rotate is not supported by OpenBaoSecretStore in R1",
        ))
    }
}
