/// Tower Layer that validates session cookies and redirects browsers to Keycloak.
use super::auth_state::{encode_auth_state, random_state, AuthState};
use super::pkce::{derive_challenge, generate_verifier};
use super::request_type::{detect, RequestType};
use super::state::BrowserLayerState;
use crate::jwks::KeyResult;
use crate::types::OidcClaims;
use cli_framework::axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{DecodingKey, Validation};
use serde_json::Value as JsonValue;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tower::{Layer, Service};

type Req = Request<Body>;
type Resp = Response;

// ── Layer ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct BrowserSessionLayer {
    pub state: Arc<BrowserLayerState>,
}

impl<S> Layer<S> for BrowserSessionLayer
where
    S: Service<Req, Response = Resp, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    type Service = BrowserSessionService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BrowserSessionService {
            inner,
            state: self.state.clone(),
        }
    }
}

// ── Service ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct BrowserSessionService<S> {
    inner: S,
    state: Arc<BrowserLayerState>,
}

impl<S> Service<Req> for BrowserSessionService<S>
where
    S: Service<Req, Response = Resp, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Resp;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Resp, std::convert::Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Req) -> Self::Future {
        let state = self.state.clone();
        let inner = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, inner);

        Box::pin(async move {
            // OPTIONS: short-circuit, no auth
            if req.method() == Method::OPTIONS {
                return inner.call(req).await;
            }

            let headers = req.headers().clone();
            let request_type = detect(&headers);
            let return_to = req
                .uri()
                .path_and_query()
                .map(|value| value.as_str())
                .unwrap_or("/");

            // Try to read and validate the session cookie
            let cookie_value = extract_cookie_header(&headers, &state.cfg.cookie_name);
            if cookie_value.is_some()
                && !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS)
                && !super::browser_origin_allowed(&headers, &state.cfg)
            {
                return Ok((StatusCode::FORBIDDEN, "Browser origin rejected").into_response());
            }

            let response: Result<Resp, std::convert::Infallible> =
                match process_session(&state, cookie_value, return_to, request_type).await {
                    SessionOutcome::Valid(claims, maybe_refresh_cookie) => {
                        if !claims.is_live() {
                            return Ok(
                                (StatusCode::UNAUTHORIZED, "Session expired").into_response()
                            );
                        }
                        req.extensions_mut().insert(claims.claims().clone());
                        req.extensions_mut().insert(*claims);
                        let mut resp = inner.call(req).await?;
                        if let Some(set_cookie) = maybe_refresh_cookie {
                            resp.headers_mut().append(
                                header::SET_COOKIE,
                                set_cookie.parse().expect("valid cookie"),
                            );
                        }
                        Ok(resp)
                    }
                    SessionOutcome::Redirect(location, clear_cookie) => {
                        let mut resp = (StatusCode::FOUND, "").into_response();
                        resp.headers_mut().append(
                            header::LOCATION,
                            location.parse().unwrap_or_else(|_| "/".parse().unwrap()),
                        );
                        if let Some(c) = clear_cookie {
                            resp.headers_mut()
                                .append(header::SET_COOKIE, c.parse().expect("valid cookie"));
                        }
                        Ok(resp)
                    }
                    SessionOutcome::Unauthorized(msg) => {
                        Ok((StatusCode::UNAUTHORIZED, msg).into_response())
                    }
                    SessionOutcome::Unavailable => Ok((
                        StatusCode::SERVICE_UNAVAILABLE,
                        "Identity service unavailable",
                    )
                        .into_response()),
                };
            let mut response = response?;
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
            Ok(response)
        })
    }
}

enum SessionOutcome {
    /// Cookie is valid (or was refreshed). Optionally includes a new Set-Cookie for token refresh.
    Valid(Box<super::BrowserSessionAccess>, Option<String>),
    /// No valid session — redirect to Keycloak (navigation) or return 401 (API).
    Redirect(String, Option<String>),
    /// API request with hard session end.
    Unauthorized(String),
    Unavailable,
}

async fn process_session(
    state: &Arc<BrowserLayerState>,
    cookie_value: Option<&str>,
    return_to: &str,
    request_type: RequestType,
) -> SessionOutcome {
    let Some(cookie) = cookie_value else {
        return redirect_to_login(state, return_to, request_type).await;
    };
    match state.authenticate(cookie).await {
        Ok(access) => {
            let header = super::session_cookie_header(&state.cfg, cookie, access.cookie_max_age());
            SessionOutcome::Valid(Box::new(access), Some(header))
        }
        Err(super::BrowserSessionError::Unavailable) => SessionOutcome::Unavailable,
        Err(_) => redirect_to_login(state, return_to, request_type).await,
    }
}
async fn redirect_to_login(
    state: &Arc<BrowserLayerState>,
    return_to: &str,
    request_type: RequestType,
) -> SessionOutcome {
    if request_type == RequestType::ApiFetch {
        return SessionOutcome::Unauthorized(r#"{"error":"unauthorized"}"#.to_string());
    }

    let verifier = generate_verifier();
    let challenge = derive_challenge(&verifier);
    let state_val = random_state();

    let return_to =
        if return_to.len() <= 1024 && super::request_type::validate_return_to(return_to).is_ok() {
            return_to.to_string()
        } else {
            "/".to_string()
        };

    let auth_state = AuthState {
        state: state_val.clone(),
        verifier,
        return_to,
    };
    let auth_state_cookie_val = encode_auth_state(&auth_state, &state.hmac_key);

    let endpoint = match state.authorization_endpoint().await {
        Ok(endpoint) => endpoint,
        Err(_) => return SessionOutcome::Unavailable,
    };
    let mut auth_url = match url::Url::parse(&endpoint) {
        Ok(url) => url,
        Err(_) => return SessionOutcome::Unavailable,
    };
    // Reserved parameters must occur exactly once even if discovery contains a query.
    let reserved = [
        "client_id",
        "redirect_uri",
        "response_type",
        "scope",
        "code_challenge",
        "code_challenge_method",
        "state",
        "nonce",
    ];
    if auth_url
        .query_pairs()
        .any(|(name, _)| reserved.contains(&name.as_ref()))
    {
        return SessionOutcome::Unavailable;
    }
    let nonce = match state.begin_login(state_val.clone()).await {
        Some(nonce) => nonce,
        None => return SessionOutcome::Unavailable,
    };
    auth_url.query_pairs_mut().extend_pairs([
        ("client_id", state.cfg.client_id.as_str()),
        ("redirect_uri", state.cfg.redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", "openid profile email"),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", state_val.as_str()),
        ("nonce", nonce.as_str()),
    ]);

    let secure_flag = super::secure_cookie_suffix(&state.cfg);

    let auth_state_cookie = format!(
        "__auth_state={}; HttpOnly{}; SameSite=Lax; Path={}; Max-Age=600",
        auth_state_cookie_val, secure_flag, state.cfg.callback_path
    );

    SessionOutcome::Redirect(auth_url.into(), Some(auth_state_cookie))
}

pub(crate) async fn validate_access_token_str(
    token: &str,
    state: &BrowserLayerState,
) -> Result<OidcClaims, String> {
    verify_access_token(token, state, true).await
}

async fn verify_access_token(
    token: &str,
    state: &BrowserLayerState,
    validate_exp: bool,
) -> Result<OidcClaims, String> {
    let header = jsonwebtoken::decode_header(token).map_err(|e| e.to_string())?;

    if !state.algorithms.contains(&header.alg) {
        return Err("unsupported algorithm".to_string());
    }

    let keys = match state.get_decoding_keys(&header.kid).await {
        KeyResult::Keys(k) => k,
        KeyResult::Unavailable => return Err("JWKS unavailable".to_string()),
        KeyResult::UnknownKid => return Err("unknown kid".to_string()),
    };

    let mut last_err = String::new();
    for key in &keys {
        match try_decode_jwt(token, key, state, validate_exp) {
            Ok(claims) => return Ok(claims),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

fn try_decode_jwt(
    token: &str,
    key: &DecodingKey,
    state: &BrowserLayerState,
    validate_exp: bool,
) -> Result<OidcClaims, String> {
    let mut validation = Validation::new(state.algorithms[0]);
    validation.algorithms = state.algorithms.clone();
    validation.set_issuer(&[&state.cfg.issuer_url]);
    validation.set_required_spec_claims(crate::jwks::REQUIRED_SPEC_CLAIMS);
    crate::jwks::apply_audience_policy(&mut validation, &state.cfg.audience);
    validation.leeway = state.cfg.clock_skew.as_secs();
    validation.validate_nbf = true;
    validation.validate_exp = validate_exp;

    let data = jsonwebtoken::decode::<JsonValue>(token, key, &validation)
        .map_err(|e| crate::jwks::map_jwt_error(&e))?;
    let c = &data.claims;

    let sub = c["sub"].as_str().ok_or("missing sub")?.to_string();
    // Required and checked above; a non-string (array) `iss` is refused.
    let iss = c["iss"].as_str().ok_or("invalid_issuer")?.to_string();
    let exp = c["exp"].as_i64().unwrap_or(0);
    let aud: Vec<String> = match &c["aud"] {
        JsonValue::String(s) => vec![s.clone()],
        JsonValue::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => vec![],
    };
    let scopes: Vec<String> = c["scope"]
        .as_str()
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default();
    let roles = crate::claim_path::ClaimPath::default_roles().strings(c);

    Ok(OidcClaims {
        sub,
        iss,
        aud,
        exp,
        iat: c["iat"].as_i64(),
        nbf: c["nbf"].as_i64(),
        preferred_username: c["preferred_username"].as_str().map(String::from),
        email: c["email"].as_str().map(String::from),
        scopes,
        roles,
        groups: Vec::new(),
        raw: c.clone(),
    })
}

pub(crate) fn extract_cookie_header<'a>(
    headers: &'a cli_framework::axum::http::HeaderMap,
    name: &str,
) -> Option<&'a str> {
    let mut found = None;
    let mut bytes = 0usize;
    for value in headers.get_all(header::COOKIE) {
        let value = value.to_str().ok()?;
        bytes = bytes.saturating_add(value.len());
        if bytes > 16 * 1024 {
            return None;
        }
        for pair in value.split(';') {
            if let Some((key, value)) = pair.trim().split_once('=') {
                if key == name {
                    if found.is_some() || value.is_empty() {
                        return None;
                    }
                    found = Some(value);
                }
            }
        }
    }
    found
}

/// Owned version of extract_cookie_header for use by dual.rs.
pub(crate) fn extract_cookie_header_owned(
    headers: &cli_framework::axum::http::HeaderMap,
    name: &str,
) -> Option<String> {
    extract_cookie_header(headers, name).map(String::from)
}
