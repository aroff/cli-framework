//! [`CallerIdentity`]: the config service's own, crate-local authentication
//! seam (spec 022).
//!
//! # Why this router authenticates itself
//!
//! `ApiServerBuilder::auth(layer)` (`src/api/mod.rs`) sets a single, global
//! auth layer applied to *every* mount, version, and (when enabled)
//! `/healthz`/`/readyz` registered on that one builder instance — there is
//! no per-mount auth in the existing API server. If the config service
//! relied on it, mounting this router into an application that also mounts
//! other, differently-authenticated routes would force all of them onto one
//! identical auth scheme. That is out of scope to fix here (a separate,
//! larger change to `src/api/`), so this module makes the config-service
//! router **self-authenticating**: [`caller_identity_layer`] wraps the
//! router this module builds with its own `axum::middleware::from_fn_with_state`,
//! independent of whatever the embedding application does — or does not — pass
//! to `ApiServerBuilder::auth()` for its other routes.
//!
//! # Why not `cli-framework-oidc` directly
//!
//! `cli-framework-oidc` already depends on `cli-framework` by path (for the
//! `auth`/`api-server` features its two halves build on); `cli-framework`
//! naming `cli-framework-oidc` back — even just its server half — would be a
//! dependency cycle, not merely undesirable coupling (the same reasoning ADR
//! 0071 already applied to keep `OidcValidator` trait-free). [`CallerIdentity`]
//! is therefore a small, object-safe, crate-local trait that never mentions
//! OIDC. An embedding application that already depends on both crates
//! supplies a one-line adapter converting `cli-framework-oidc`'s validated
//! `OidcClaims` into this trait's `serde_json::Value` shape — see
//! `skill/examples/with_config_service/src/main.rs` for a real one, not just
//! a sketch: spec 022 requires a runnable adapter, since that is what proves
//! the seam actually composes end to end.

use super::assignment::rule_matches;
use super::error::ConfigServiceError;
use super::types::AssignmentRule;
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{FromRequestParts, State};
use axum::http::{header::AUTHORIZATION, request::Parts, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::sync::Arc;

/// Validated claims from whatever authentication scheme the embedding
/// application configured.
///
/// One method, object-safe, `async`, taking the raw `Authorization` header
/// value and returning the validated claims as `serde_json::Value` — JSON
/// rather than a concrete claims type is deliberate: assignment-rule
/// evaluation (spec 022, "Assignment rule shape") walks arbitrary
/// dot-separated claim paths (`realm_access.roles`, or anything else an
/// identity provider happens to name), and a JSON value is the one shape
/// every possible claims representation can be converted into without this
/// crate needing to know what any of them look like.
#[async_trait]
pub trait CallerIdentity: Send + Sync {
    /// Validate `authorization_header` (e.g. `Some("Bearer <token>")`) and
    /// return the validated claims. Returns
    /// [`ConfigServiceError::MissingCredential`] when `None` was passed (no
    /// `Authorization` header on the request) and
    /// [`ConfigServiceError::InvalidCredential`] for any other rejection —
    /// the router maps both to `401` uniformly; see [`ConfigServiceError`].
    async fn authenticate(
        &self,
        authorization_header: Option<&str>,
    ) -> Result<serde_json::Value, ConfigServiceError>;
}

/// The validated claims for the current request, inserted into the request
/// extension map by [`require_caller_identity`]. Handlers extract it with
/// the ordinary axum `Extension`-style pattern via
/// [`FromRequestParts`] — implemented below so a handler can simply take
/// `CallerClaims` as a parameter rather than reaching into extensions by
/// hand.
#[derive(Debug, Clone)]
pub struct CallerClaims(pub serde_json::Value);

/// Rejection when a handler extracts [`CallerClaims`] on a route that
/// somehow isn't behind [`require_caller_identity`] — should be unreachable
/// through [`super::router::config_service_router`], which always applies
/// the middleware to every route it registers, but implemented defensively
/// rather than via `.unwrap()`/`.expect()` on a missing extension.
pub struct MissingCallerClaims;

impl IntoResponse for MissingCallerClaims {
    fn into_response(self) -> Response {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "config service auth middleware not installed on this route",
        )
            .into_response()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for CallerClaims {
    type Rejection = MissingCallerClaims;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<CallerClaims>()
            .cloned()
            .ok_or(MissingCallerClaims)
    }
}

/// `axum::middleware::from_fn_with_state` handler: validates every request
/// against `identity` before any route handler runs, rejecting with `401`
/// on failure. Wired onto every route by
/// [`super::router::config_service_router`] — see that module.
pub async fn require_caller_identity(
    State(identity): State<Arc<dyn CallerIdentity>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    match identity.authenticate(header).await {
        Ok(claims) => {
            req.extensions_mut().insert(CallerClaims(claims));
            next.run(req).await
        }
        Err(e) => e.into_response(),
    }
}

/// `axum::middleware::from_fn_with_state` handler: the second half of spec
/// 023's two-gate model for `/v1/admin/*` routes. Must run **after**
/// [`require_caller_identity`] has already inserted [`CallerClaims`] into the
/// request extensions — see [`super::admin_router`]'s module docs for the
/// exact `.layer()` ordering that guarantees this (axum/tower's `.layer()`
/// wraps outward-in, so the identity layer must be the *last* `.layer()`
/// call, making it the *first* middleware to run).
///
/// Evaluates `admin_rule` against the caller's already-validated claims with
/// [`rule_matches`] — the identical function
/// [`super::assignment::resolve_profile`] uses for ordinary profile
/// assignment, per spec 023's explicit requirement to reuse "the identical
/// `{claim_path, operator, value}` shape" rather than a second copy of rule
/// evaluation. A match proceeds to the next handler; no match is `403`
/// (distinct from [`require_caller_identity`]'s `401` — "who are you" vs
/// "you may not do this").
///
/// If [`CallerClaims`] is somehow missing from the request extensions (the
/// layering-order bug this module's docs warn about), this responds `500`
/// rather than panicking — the same defensive posture
/// [`MissingCallerClaims`] already takes for handlers extracting
/// [`CallerClaims`] directly.
pub async fn require_admin_role(
    State(admin_rule): State<Arc<AssignmentRule>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(claims) = req.extensions().get::<CallerClaims>().cloned() else {
        return MissingCallerClaims.into_response();
    };
    if rule_matches(&admin_rule, &claims.0) {
        next.run(req).await
    } else {
        (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "caller does not satisfy the administrative role rule" })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysDenies;

    #[async_trait]
    impl CallerIdentity for AlwaysDenies {
        async fn authenticate(
            &self,
            _authorization_header: Option<&str>,
        ) -> Result<serde_json::Value, ConfigServiceError> {
            Err(ConfigServiceError::MissingCredential)
        }
    }

    struct AlwaysAllows;

    #[async_trait]
    impl CallerIdentity for AlwaysAllows {
        async fn authenticate(
            &self,
            _authorization_header: Option<&str>,
        ) -> Result<serde_json::Value, ConfigServiceError> {
            Ok(serde_json::json!({"sub": "u1"}))
        }
    }

    #[tokio::test]
    async fn always_denies_rejects_every_header_shape() {
        let id = AlwaysDenies;
        assert!(id.authenticate(None).await.is_err());
        assert!(id.authenticate(Some("Bearer x")).await.is_err());
    }

    #[tokio::test]
    async fn always_allows_returns_claims() {
        let id = AlwaysAllows;
        let claims = id.authenticate(Some("Bearer x")).await.unwrap();
        assert_eq!(claims["sub"], "u1");
    }

    #[test]
    fn missing_caller_claims_responds_500() {
        let resp = MissingCallerClaims.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn config_service_error_responds_401_with_www_authenticate() {
        let resp = ConfigServiceError::MissingCredential.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert!(resp.headers().get("www-authenticate").is_some());

        let resp = ConfigServiceError::InvalidCredential("bad".into()).into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    }
}
