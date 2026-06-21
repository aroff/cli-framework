/// Dual-mode API layer: accepts Bearer JWT (Agents) or Session Cookie (browser fetch).
///
/// Bearer takes precedence. If Bearer is present but invalid, the request is rejected
/// with 401 — the cookie is NOT consulted as a fallback.
use super::layer::{extract_cookie_header_owned, validate_jwt_from_cookie};
use super::state::BrowserLayerState;
use crate::jwks::KeyResult;
use crate::types::OidcClaims;
use cli_framework::axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

type Req = Request<Body>;

#[derive(Clone)]
pub(crate) struct DualModeLayer {
    pub state: Arc<BrowserLayerState>,
}

impl<S> Layer<S> for DualModeLayer
where
    S: Service<Req, Response = Response, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    type Service = DualModeService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        DualModeService {
            inner,
            state: self.state.clone(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct DualModeService<S> {
    inner: S,
    state: Arc<BrowserLayerState>,
}

impl<S> Service<Req> for DualModeService<S>
where
    S: Service<Req, Response = Response, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, std::convert::Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Req) -> Self::Future {
        let state = self.state.clone();
        let inner = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, inner);

        Box::pin(async move {
            let headers = req.headers().clone();

            // 1. Try Bearer header first
            if let Some(bearer) = extract_bearer(&headers) {
                match validate_bearer_token(bearer, &state).await {
                    Ok(claims) => {
                        req.extensions_mut().insert(claims);
                        return inner.call(req).await;
                    }
                    Err(resp) => return Ok(resp),
                }
            }

            // 2. No Bearer — try session cookie
            let cookie_val = extract_cookie_header_owned(&headers, &state.cfg.cookie_name);
            if let Some(ref cv) = cookie_val {
                match validate_jwt_from_cookie(cv, &state).await {
                    Ok(claims) => {
                        req.extensions_mut().insert(claims);
                        return inner.call(req).await;
                    }
                    Err(_) => {
                        return Ok((StatusCode::UNAUTHORIZED, r#"{"error":"unauthorized"}"#)
                            .into_response());
                    }
                }
            }

            // 3. Neither present
            Ok((
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                r#"{"error":"unauthorized"}"#,
            )
                .into_response())
        })
    }
}

fn extract_bearer(headers: &cli_framework::axum::http::HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    if v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer ") {
        Some(&v[7..])
    } else {
        None
    }
}

async fn validate_bearer_token(
    token: &str,
    state: &Arc<BrowserLayerState>,
) -> Result<OidcClaims, Response> {
    use cli_framework::axum::response::IntoResponse;
    use jsonwebtoken::Validation;
    use serde_json::Value as JsonValue;

    let header = jsonwebtoken::decode_header(token).map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer error=\"invalid_token\"")],
            "",
        )
            .into_response()
    })?;

    if !state.algorithms.contains(&header.alg) {
        return Err((
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                "Bearer error=\"invalid_token\", error_description=\"unsupported_algorithm\"",
            )],
            "",
        )
            .into_response());
    }

    let keys = match state.get_decoding_keys(&header.kid).await {
        KeyResult::Keys(k) => k,
        KeyResult::Unavailable => {
            return Err(StatusCode::SERVICE_UNAVAILABLE.into_response());
        }
        KeyResult::UnknownKid => {
            return Err((
                StatusCode::UNAUTHORIZED,
                [(
                    header::WWW_AUTHENTICATE,
                    "Bearer error=\"invalid_token\", error_description=\"unknown_key\"",
                )],
                "",
            )
                .into_response());
        }
    };

    let mut last_err = String::new();
    for key in &keys {
        let mut val = Validation::new(state.algorithms[0]);
        val.algorithms = state.algorithms.clone();
        val.set_issuer(&[&state.cfg.issuer_url]);
        match &state.api_audience {
            crate::types::AudiencePolicy::Require(a) => val.set_audience(&[a]),
            crate::types::AudiencePolicy::RequireAny(a) => val.set_audience(a),
            crate::types::AudiencePolicy::Unchecked => val.validate_aud = false,
        }
        val.leeway = state.cfg.clock_skew.as_secs();

        match jsonwebtoken::decode::<JsonValue>(token, key, &val) {
            Ok(data) => {
                let c = &data.claims;
                let sub = match c["sub"].as_str() {
                    Some(s) => s.to_string(),
                    None => {
                        last_err = "malformed_token".to_string();
                        continue;
                    }
                };
                let aud: Vec<String> = match &c["aud"] {
                    JsonValue::String(s) => vec![s.clone()],
                    JsonValue::Array(a) => a
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                    _ => vec![],
                };
                return Ok(OidcClaims {
                    sub,
                    iss: c["iss"].as_str().unwrap_or("").to_string(),
                    aud,
                    exp: c["exp"].as_i64().unwrap_or(0),
                    iat: c["iat"].as_i64(),
                    nbf: c["nbf"].as_i64(),
                    preferred_username: c["preferred_username"].as_str().map(String::from),
                    email: c["email"].as_str().map(String::from),
                    scopes: c["scope"]
                        .as_str()
                        .map(|s| s.split_whitespace().map(String::from).collect())
                        .unwrap_or_default(),
                    roles: c["realm_access"]["roles"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    raw: c.clone(),
                });
            }
            Err(e) => last_err = crate::jwks::map_jwt_error(&e),
        }
    }

    Err((
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            format!("Bearer error=\"invalid_token\", error_description=\"{last_err}\""),
        )],
        "",
    )
        .into_response())
}
