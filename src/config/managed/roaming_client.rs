//! [`RoamingConfigClient`]: `GET`/`PUT /v1/config/{app}` for the user-scoped
//! roaming document (spec 021, "Roaming user config").
//!
//! Whole-document, with `If-Match` optimistic concurrency — and, unlike
//! [`super::PolicyClient`], no elaborate failure-mapping table: this is a
//! plain authenticated read/write of a document the *user* owns, not an
//! organisation-authored control channel, so an ordinary error is just an
//! error. The one hard rule spec 021 does call out is enforced client-side
//! regardless of what the caller passes in: fields not declared `scope:
//! user` in the manifest are never sent, full stop.

use crate::auth::AuthenticatedHttpClient;
use crate::config::manifest::{ConfigManifest, Scope};
use reqwest::header::{ETAG, IF_MATCH};
use reqwest::StatusCode;
use serde_json::{Map, Value};
use std::sync::Arc;

/// A roaming document as returned by `GET /v1/config/{app}`: the flat,
/// dotted-path-keyed value (same coordinate system as
/// [`crate::config::Policy`]'s trees) plus the ETag needed to write it back
/// with [`RoamingConfigClient::put`].
#[derive(Debug, Clone, PartialEq)]
pub struct RoamingDocument {
    pub value: Map<String, Value>,
    pub etag: Option<String>,
}

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum RoamingClientError {
    /// `412 Precondition Failed`: the document moved server-side since the
    /// `If-Match` value being sent was read.
    #[error("roaming config write rejected: If-Match no longer matches the server's document")]
    Conflict,
    #[error("roaming config request failed: {0}")]
    Request(String),
    #[error("roaming config response could not be parsed: {0}")]
    InvalidResponse(String),
}

/// Restrict `doc` to only the keys the manifest declares `scope: user` for,
/// excluding any that are `local_only` or `secret`.
///
/// Scope alone is not enough. A `local_only` field is bootstrap state for one
/// machine — an install identifier, a service address — and roaming it makes
/// several installations look like one. A `secret` may never leave the
/// machine at all. Both exclusions are unconditional: a caller cannot opt out.
pub fn filter_user_scoped(
    manifest: &ConfigManifest,
    doc: &Map<String, Value>,
) -> Map<String, Value> {
    let user_paths: std::collections::HashSet<String> = manifest
        .iter_leaves()
        .into_iter()
        .filter(|l| l.field.scope == Scope::User && !l.field.local_only && !l.field.secret)
        .map(|l| l.path)
        .collect();
    doc.iter()
        .filter(|(k, _)| user_paths.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

pub struct RoamingConfigClient {
    http: Arc<AuthenticatedHttpClient>,
    base_url: String,
    app: String,
}

impl RoamingConfigClient {
    pub fn new(
        http: Arc<AuthenticatedHttpClient>,
        base_url: impl Into<String>,
        app: impl Into<String>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            app: app.into(),
        }
    }

    fn url(&self) -> String {
        format!(
            "{}/v1/config/{}",
            self.base_url.trim_end_matches('/'),
            self.app
        )
    }

    /// Read the current roaming document — called at startup (spec 021).
    pub async fn get(&self) -> Result<RoamingDocument, RoamingClientError> {
        let client = self.http.client().clone();
        let url = self.url();
        let build = move || client.get(&url);

        let resp = self
            .http
            .execute_with_retry(build)
            .await
            .map_err(|e| RoamingClientError::Request(e.to_string()))?;

        let etag = resp
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| RoamingClientError::InvalidResponse(e.to_string()))?;
        let value: Map<String, Value> = if bytes.is_empty() {
            Map::new()
        } else {
            serde_json::from_slice(&bytes)
                .map_err(|e| RoamingClientError::InvalidResponse(e.to_string()))?
        };
        Ok(RoamingDocument { value, etag })
    }

    /// Write the roaming document — called whenever a `scope: user` field
    /// changes. `doc` is filtered to `scope: user` fields via
    /// [`filter_user_scoped`] before anything is sent, and the write is
    /// conditioned on `if_match` via the `If-Match` header: a `412` from the
    /// server (the document moved since `if_match` was read) surfaces as
    /// [`RoamingClientError::Conflict`] rather than silently overwriting a
    /// concurrent edit.
    pub async fn put(
        &self,
        manifest: &ConfigManifest,
        doc: &Map<String, Value>,
        if_match: &str,
    ) -> Result<(), RoamingClientError> {
        let filtered = filter_user_scoped(manifest, doc);

        let client = self.http.client().clone();
        let url = self.url();
        let if_match_value = if_match.to_string();
        let body = Value::Object(filtered);
        let build = move || {
            client
                .put(&url)
                .header(IF_MATCH, if_match_value.as_str())
                .json(&body)
        };

        let resp = self
            .http
            .execute_with_retry(build)
            .await
            .map_err(|e| classify_put_error(&e))?;

        match resp.status() {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            other => Err(RoamingClientError::Request(format!(
                "unexpected HTTP status {other}"
            ))),
        }
    }
}

fn classify_put_error(e: &anyhow::Error) -> RoamingClientError {
    if let Some(re) = e.downcast_ref::<reqwest::Error>() {
        if re.status() == Some(StatusCode::PRECONDITION_FAILED) {
            return RoamingClientError::Conflict;
        }
    }
    RoamingClientError::Request(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{FieldKind, FieldManifest};
    use serde_json::json;

    fn leaf(key: &str, scope: Scope) -> FieldManifest {
        FieldManifest {
            key: key.to_string(),
            kind: FieldKind::Str,
            default: None,
            label: None,
            description: None,
            group: None,
            scope,
            platforms: vec![],
            secret: false,
            local_only: false,
            protected: false,
            manageable: true,
            enforceable: true,
            restart_required: false,
            constraints: None,
        }
    }

    #[test]
    fn filter_user_scoped_keeps_only_user_fields() {
        let manifest = ConfigManifest::new(
            "app",
            vec![
                leaf("nickname", Scope::User),
                leaf("install_id", Scope::Machine),
                leaf("compliance_flag", Scope::Org),
            ],
        );
        let mut doc = Map::new();
        doc.insert("nickname".to_string(), json!("alice"));
        doc.insert("install_id".to_string(), json!("machine-123"));
        doc.insert("compliance_flag".to_string(), json!(true));
        doc.insert("unknown_field".to_string(), json!("x"));

        let filtered = filter_user_scoped(&manifest, &doc);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered.get("nickname"), Some(&json!("alice")));
    }

    #[test]
    fn filter_user_scoped_on_empty_manifest_drops_everything() {
        let manifest = ConfigManifest::new("app", vec![]);
        let mut doc = Map::new();
        doc.insert("anything".to_string(), json!(1));
        assert!(filter_user_scoped(&manifest, &doc).is_empty());
    }
}
