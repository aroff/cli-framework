/// Tower Layer that validates session cookies and redirects browsers to Keycloak.
use super::auth_state::{encode_auth_state, random_state, AuthState};
use super::cookie::{decrypt_cookie, encrypt_cookie, CookieError};
use super::handlers::refresh_tokens;
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
use std::time::{SystemTime, UNIX_EPOCH};
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

            // Try to read and validate the session cookie
            let cookie_value = extract_cookie_header(&headers, &state.cfg.cookie_name);

            match process_session(&state, cookie_value, &headers, request_type).await {
                SessionOutcome::Valid(claims, maybe_refresh_cookie) => {
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
            }
        })
    }
}

enum SessionOutcome {
    /// Cookie is valid (or was refreshed). Optionally includes a new Set-Cookie for token refresh.
    Valid(Box<OidcClaims>, Option<String>),
    /// No valid session — redirect to Keycloak (navigation) or return 401 (API).
    Redirect(String, Option<String>),
    /// API request with hard session end.
    Unauthorized(String),
}

async fn process_session(
    state: &Arc<BrowserLayerState>,
    cookie_value: Option<&str>,
    headers: &cli_framework::axum::http::HeaderMap,
    request_type: RequestType,
) -> SessionOutcome {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let cookie_value = match cookie_value {
        Some(v) => v,
        None => return redirect_to_login(state, headers, request_type),
    };

    let payload = match decrypt_cookie(state.cfg.session_key.as_bytes(), cookie_value) {
        Ok(p) => p,
        Err(CookieError::UnknownVersion(_))
        | Err(CookieError::Invalid)
        | Err(CookieError::Tampered) => {
            return redirect_to_login(state, headers, request_type);
        }
        Err(_) => return redirect_to_login(state, headers, request_type),
    };

    // Check refresh token expiry (hard session boundary)
    let refresh_skew = state.cfg.clock_skew.as_secs() as i64;
    if now_secs > payload.refresh_exp + refresh_skew {
        tracing::info!(event = "session_expired");
        return match request_type {
            RequestType::Navigation => redirect_to_login(state, headers, request_type),
            RequestType::ApiFetch => {
                SessionOutcome::Unauthorized(r#"{"error":"session_expired"}"#.to_string())
            }
        };
    }

    // Validate the access token JWT
    match validate_access_token(&payload.access_token, state).await {
        Ok(claims) => {
            // Check if near expiry — proactive refresh
            let access_exp = claims.exp;
            let refresh_skew_secs = state.cfg.refresh_skew.as_secs() as i64;
            if now_secs + refresh_skew_secs > access_exp {
                // Attempt in-handler token refresh
                let token_ep = state.token_endpoint().await;
                match refresh_tokens(
                    &state.http,
                    &token_ep,
                    &state.cfg.client_id,
                    &payload.refresh_token,
                )
                .await
                {
                    Ok(new_tokens) => {
                        tracing::info!(event = "refresh", sub = %claims.sub);
                        let new_exp = new_tokens.refresh_expires_in;
                        let new_refresh_exp = now_secs + new_exp as i64;
                        let new_cookie = encrypt_cookie(
                            state.cfg.session_key.as_bytes(),
                            &new_tokens.access_token,
                            &new_tokens.refresh_token,
                            new_refresh_exp,
                        )
                        .ok();

                        // Re-validate the new access token to get fresh claims
                        let new_claims = match validate_access_token_str(
                            &new_tokens.access_token,
                            state,
                        )
                        .await
                        {
                            Ok(c) => c,
                            Err(_) => claims, // fallback to old claims
                        };

                        let set_cookie_header = new_cookie.map(|v| {
                            build_session_cookie_header(state, &v, new_refresh_exp, now_secs)
                        });
                        SessionOutcome::Valid(Box::new(new_claims), set_cookie_header)
                    }
                    Err(e) => {
                        tracing::warn!(event = "refresh_failure", error = e);
                        // Refresh failed — still valid for this request since the access token
                        // is near-expiry but not yet expired (within clock_skew)
                        SessionOutcome::Valid(Box::new(claims), None)
                    }
                }
            } else {
                SessionOutcome::Valid(Box::new(claims), None)
            }
        }
        Err(_) => {
            // Access token is invalid/expired — try refresh
            let token_ep = state.token_endpoint().await;
            match refresh_tokens(
                &state.http,
                &token_ep,
                &state.cfg.client_id,
                &payload.refresh_token,
            )
            .await
            {
                Ok(new_tokens) => {
                    let new_exp = new_tokens.refresh_expires_in;
                    let new_refresh_exp = now_secs + new_exp as i64;
                    let new_cookie = encrypt_cookie(
                        state.cfg.session_key.as_bytes(),
                        &new_tokens.access_token,
                        &new_tokens.refresh_token,
                        new_refresh_exp,
                    )
                    .ok();
                    match validate_access_token_str(&new_tokens.access_token, state).await {
                        Ok(claims) => {
                            tracing::info!(event = "refresh", sub = %claims.sub);
                            let set_cookie = new_cookie.map(|v| {
                                build_session_cookie_header(state, &v, new_refresh_exp, now_secs)
                            });
                            SessionOutcome::Valid(Box::new(claims), set_cookie)
                        }
                        Err(_) => redirect_to_login(state, headers, request_type),
                    }
                }
                Err(e) => {
                    tracing::warn!(event = "refresh_failure", error = e);
                    match request_type {
                        RequestType::Navigation => redirect_to_login(state, headers, request_type),
                        RequestType::ApiFetch => SessionOutcome::Unauthorized(
                            r#"{"error":"session_expired"}"#.to_string(),
                        ),
                    }
                }
            }
        }
    }
}

fn redirect_to_login(
    state: &Arc<BrowserLayerState>,
    headers: &cli_framework::axum::http::HeaderMap,
    request_type: RequestType,
) -> SessionOutcome {
    if request_type == RequestType::ApiFetch {
        return SessionOutcome::Unauthorized(r#"{"error":"unauthorized"}"#.to_string());
    }

    let verifier = generate_verifier();
    let challenge = derive_challenge(&verifier);
    let state_val = random_state();

    // Determine return_to from the request path (not available here — callers pass headers only)
    let return_to = extract_original_path(headers).unwrap_or_else(|| "/".to_string());

    let auth_state = AuthState {
        state: state_val.clone(),
        verifier,
        return_to,
    };
    let auth_state_cookie_val = encode_auth_state(&auth_state, &state.hmac_key);

    let auth_url = format!(
        "{}/protocol/openid-connect/auth?client_id={}&redirect_uri={}&response_type=code&scope=openid+profile+email&code_challenge={}&code_challenge_method=S256&state={}",
        state.cfg.issuer_url,
        url_encode(&state.cfg.client_id),
        url_encode(&state.cfg.redirect_uri),
        url_encode(&challenge),
        url_encode(&state_val),
    );

    let is_loopback = state.cfg.redirect_uri.starts_with("http://127.0.0.1")
        || state.cfg.redirect_uri.starts_with("http://localhost");
    let secure_flag = if is_loopback { "" } else { "; Secure" };

    let auth_state_cookie = format!(
        "__auth_state={}; HttpOnly{}; SameSite=Lax; Path=/callback; Max-Age=600",
        auth_state_cookie_val, secure_flag
    );

    SessionOutcome::Redirect(auth_url, Some(auth_state_cookie))
}

fn extract_original_path(headers: &cli_framework::axum::http::HeaderMap) -> Option<String> {
    // In practice the path is in the request URI, but since we only have headers here,
    // we use a sensible default. The caller (BrowserSessionService) can be enhanced
    // to pass the path; for now return None so the handler defaults to "/".
    let _ = headers;
    None
}

async fn validate_access_token(
    token: &str,
    state: &BrowserLayerState,
) -> Result<OidcClaims, String> {
    validate_access_token_str(token, state).await
}

async fn validate_access_token_str(
    token: &str,
    state: &BrowserLayerState,
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
        match try_decode_jwt(token, key, state) {
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
) -> Result<OidcClaims, String> {
    let mut validation = Validation::new(state.algorithms[0]);
    validation.algorithms = state.algorithms.clone();
    validation.set_issuer(&[&state.cfg.issuer_url]);
    match &state.cfg.audience {
        crate::types::AudiencePolicy::Require(aud) => validation.set_audience(&[aud]),
        crate::types::AudiencePolicy::RequireAny(auds) => validation.set_audience(auds),
        crate::types::AudiencePolicy::Unchecked => validation.validate_aud = false,
    }
    validation.leeway = state.cfg.clock_skew.as_secs();

    let data = jsonwebtoken::decode::<JsonValue>(token, key, &validation)
        .map_err(|e| crate::jwks::map_jwt_error(&e))?;
    let c = &data.claims;

    let sub = c["sub"].as_str().ok_or("missing sub")?.to_string();
    let iss = c["iss"].as_str().unwrap_or("").to_string();
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
    let roles: Vec<String> = c["realm_access"]["roles"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

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
        raw: c.clone(),
    })
}

fn build_session_cookie_header(
    state: &BrowserLayerState,
    cookie_value: &str,
    refresh_exp: i64,
    now_secs: i64,
) -> String {
    let from_exp = (refresh_exp - now_secs).max(0) as u64;
    let max_age = from_exp.min(state.cfg.session_ttl.as_secs());
    let is_loopback = state.cfg.redirect_uri.starts_with("http://127.0.0.1")
        || state.cfg.redirect_uri.starts_with("http://localhost");
    let secure = if is_loopback { "" } else { "; Secure" };
    format!(
        "{}={}; HttpOnly{}; SameSite=Lax; Path=/; Max-Age={}",
        state.cfg.cookie_name, cookie_value, secure, max_age
    )
}

fn extract_cookie_header<'a>(
    headers: &'a cli_framework::axum::http::HeaderMap,
    name: &str,
) -> Option<&'a str> {
    let v = headers.get(header::COOKIE)?;
    let s = v.to_str().ok()?;
    for pair in s.split(';') {
        let pair = pair.trim();
        if let Some(rest) = pair.strip_prefix(name) {
            if let Some(val) = rest.strip_prefix('=') {
                return Some(val);
            }
        }
    }
    None
}

/// Owned version of extract_cookie_header for use by dual.rs.
pub(crate) fn extract_cookie_header_owned(
    headers: &cli_framework::axum::http::HeaderMap,
    name: &str,
) -> Option<String> {
    extract_cookie_header(headers, name).map(String::from)
}

/// Validate an access token stored inside a decrypted session cookie.
pub(crate) async fn validate_jwt_from_cookie(
    cookie_value: &str,
    state: &BrowserLayerState,
) -> Result<OidcClaims, String> {
    let payload = decrypt_cookie(state.cfg.session_key.as_bytes(), cookie_value)
        .map_err(|e| e.to_string())?;

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if now_secs > payload.refresh_exp + state.cfg.clock_skew.as_secs() as i64 {
        return Err("session_expired".to_string());
    }

    validate_access_token_str(&payload.access_token, state).await
}

fn url_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            c => format!("%{:02X}", c as u32).chars().collect(),
        })
        .collect()
}
