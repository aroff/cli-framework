//! [`config_service_router`]: the axum router spec 022 requires the
//! embedding application to mount at a prefix of its choosing.
//!
//! Every route below is wrapped in its own
//! [`super::identity::require_caller_identity`] middleware — see that
//! module's docs for why this router authenticates itself rather than
//! relying on `ApiServerBuilder::auth()`.

use super::error::UserConfigWriteError;
use super::identity::{require_caller_identity, CallerClaims};
use super::state::{ConfigServiceState, PolicyLookupError};
use axum::extract::{Path, State};
use axum::http::header::{ETAG, IF_MATCH, IF_NONE_MATCH};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// Build the config-service router. Mount it into your own
/// `ApiServerBuilder` (or any axum `Router`) at whatever prefix you like:
///
/// ```no_run
/// # use cli_framework::api::ApiServerBuilder;
/// # use cli_framework::config::service::config_service_router;
/// # fn wire(state: std::sync::Arc<cli_framework::config::service::ConfigServiceState>) {
/// let server = ApiServerBuilder::new()
///     .mount("/config", config_service_router(state))
///     // ... .version(...) etc.
///     .build();
/// # }
/// ```
///
/// Call [`ConfigServiceState::validate_at_startup`] before this and refuse
/// to start the process if it returns an error — this function does not do
/// that for you, since a panic/exit policy on validation failure is an
/// application-level decision (spec 022 user story 27).
pub fn config_service_router(state: Arc<ConfigServiceState>) -> Router {
    let identity = state.identity.clone();
    Router::new()
        .route("/v1/policy/{app}", get(get_policy))
        .route("/v1/manifest/{app}", get(get_manifest))
        .route(
            "/v1/config/{app}",
            get(get_user_config).put(put_user_config),
        )
        .route("/v1/resolve/{app}", get(get_resolve))
        .layer(axum::middleware::from_fn_with_state(
            identity,
            require_caller_identity,
        ))
        .with_state(state)
}

fn internal_error(context: &str, err: impl std::fmt::Display) -> Response {
    tracing::error!("config-service: {context}: {err}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

fn unmanaged() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "unmanaged" }))).into_response()
}

/// The subject identifier used to key roaming user documents: the
/// validated claims' `sub`. A judgment call — spec 022 does not name a
/// specific claim as the roaming-document key, but `sub` is the one claim
/// every OIDC-shaped identity (user or service account) carries, and it is
/// exactly what `cli-framework-oidc`'s `OidcClaims.sub` already surfaces.
fn subject_from_claims(claims: &Value) -> Option<String> {
    claims
        .get("sub")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn missing_sub_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "validated identity is missing a 'sub' claim" })),
    )
        .into_response()
}

async fn get_policy(
    State(state): State<Arc<ConfigServiceState>>,
    Path(app): Path<String>,
    CallerClaims(claims): CallerClaims,
    headers: HeaderMap,
) -> Response {
    match state.lookup_policy(&app, &claims).await {
        Ok(policy) => {
            let etag = format!("\"{}\"", policy.policy_version);
            let if_none_match = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok());
            if if_none_match == Some(etag.as_str()) {
                return (StatusCode::NOT_MODIFIED, [(ETAG, etag)]).into_response();
            }
            (StatusCode::OK, [(ETAG, etag)], Json(policy)).into_response()
        }
        Err(PolicyLookupError::Unmanaged) => unmanaged(),
        Err(PolicyLookupError::Internal(msg)) => internal_error("policy lookup", msg),
    }
}

async fn get_manifest(
    State(state): State<Arc<ConfigServiceState>>,
    Path(app): Path<String>,
    CallerClaims(_claims): CallerClaims,
) -> Response {
    match state.policy_store.manifest(&app).await {
        Ok(Some(manifest)) => (StatusCode::OK, Json(manifest.doc)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => internal_error("manifest lookup", e),
    }
}

async fn get_resolve(
    State(state): State<Arc<ConfigServiceState>>,
    Path(app): Path<String>,
    CallerClaims(claims): CallerClaims,
) -> Response {
    match state.resolve_diagnostic(&app, &claims).await {
        Ok(diag) => (StatusCode::OK, Json(diag)).into_response(),
        Err(PolicyLookupError::Unmanaged) => unmanaged(),
        Err(PolicyLookupError::Internal(msg)) => internal_error("resolve diagnostic", msg),
    }
}

async fn get_user_config(
    State(state): State<Arc<ConfigServiceState>>,
    Path(app): Path<String>,
    CallerClaims(claims): CallerClaims,
) -> Response {
    let Some(subject) = subject_from_claims(&claims) else {
        return missing_sub_response();
    };
    match state.user_config_store.get(&app, &subject).await {
        Ok(doc) => {
            let etag = format!("\"{}\"", doc.version);
            (StatusCode::OK, [(ETAG, etag)], Json(Value::Object(doc.doc))).into_response()
        }
        Err(e) => internal_error("user config lookup", e),
    }
}

/// Everything a submitted roaming-config field can be rejected for on
/// write — a security-relevant, server-authoritative check independent of
/// (and not merely trusting) the client's own
/// `RoamingConfigClient::put`/`filter_user_scoped` filtering (spec 022 user
/// story 24: "machine-scoped and secret fields rejected on write").
fn first_invalid_user_field(
    manifest: &crate::config::manifest::ConfigManifest,
    doc: &Map<String, Value>,
) -> Option<String> {
    use crate::config::manifest::Scope;
    for key in doc.keys() {
        match manifest.leaf_by_path(key) {
            None => return Some(format!("unknown field '{key}'")),
            Some(field) => {
                if field.secret {
                    return Some(format!("field '{key}' is secret and cannot be written"));
                }
                if field.scope != Scope::User {
                    return Some(format!("field '{key}' is not user-scoped"));
                }
            }
        }
    }
    None
}

fn parse_etag(raw: &str) -> Option<u64> {
    raw.trim().trim_matches('"').parse().ok()
}

async fn put_user_config(
    State(state): State<Arc<ConfigServiceState>>,
    Path(app): Path<String>,
    CallerClaims(claims): CallerClaims,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(subject) = subject_from_claims(&claims) else {
        return missing_sub_response();
    };

    let Some(if_match_raw) = headers.get(IF_MATCH).and_then(|v| v.to_str().ok()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "If-Match header is required" })),
        )
            .into_response();
    };
    let Some(expected_version) = parse_etag(if_match_raw) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "If-Match header could not be parsed" })),
        )
            .into_response();
    };

    let Value::Object(doc) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "request body must be a JSON object" })),
        )
            .into_response();
    };

    let size = serde_json::to_vec(&doc)
        .map(|b| b.len())
        .unwrap_or(usize::MAX);
    if size > state.max_user_config_bytes {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({
                "error": format!(
                    "document is {size} bytes, exceeding the {} byte limit",
                    state.max_user_config_bytes
                )
            })),
        )
            .into_response();
    }

    match state.policy_store.manifest(&app).await {
        Ok(Some(manifest)) => {
            if let Some(problem) = first_invalid_user_field(&manifest.doc, &doc) {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": problem })))
                    .into_response();
            }
        }
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal_error("manifest lookup for write validation", e),
    }

    match state
        .user_config_store
        .put(&app, &subject, doc, expected_version)
        .await
    {
        Ok(new_version) => (StatusCode::OK, [(ETAG, format!("\"{new_version}\""))]).into_response(),
        Err(UserConfigWriteError::Conflict { .. }) => {
            StatusCode::PRECONDITION_FAILED.into_response()
        }
        Err(UserConfigWriteError::Store(e)) => internal_error("user config write", e),
    }
}
