/// Axum handlers for /callback and /logout routes.
use super::auth_state::{decode_auth_state, AuthState};
use super::layer::extract_cookie_header as extract_cookie_value;
use super::state::BrowserLayerState;
use cli_framework::axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::sync::Arc;

// ── Callback handler ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

pub async fn handle_callback(
    State(state): State<Arc<BrowserLayerState>>,
    Query(params): Query<CallbackParams>,
    headers: cli_framework::axum::http::HeaderMap,
) -> Response {
    let mut response = callback_response(&state, params, &headers).await;
    response.headers_mut().append(
        header::SET_COOKIE,
        clear_cookie("__auth_state", &state.cfg.callback_path, &state.cfg)
            .parse()
            .expect("validated cookie"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::PRAGMA, "no-cache".parse().unwrap());
    response
}

async fn callback_response(
    state: &Arc<BrowserLayerState>,
    params: CallbackParams,
    headers: &cli_framework::axum::http::HeaderMap,
) -> Response {
    // Treat provider-supplied error text as untrusted and potentially sensitive.
    let returned_state = match params.state {
        Some(ref s) => s.clone(),
        None => {
            return (StatusCode::BAD_REQUEST, "Missing state parameter").into_response();
        }
    };

    // Read and verify __auth_state cookie
    let auth_state_value = match extract_cookie_value(headers, "__auth_state") {
        Some(v) => v,
        None => {
            return (StatusCode::BAD_REQUEST, "Missing auth state cookie").into_response();
        }
    };

    let auth_state: AuthState = match decode_auth_state(auth_state_value, &state.hmac_key) {
        Some(s) => s,
        None => {
            tracing::warn!(event = "auth_state_invalid");
            return (StatusCode::BAD_REQUEST, "Invalid or expired auth state").into_response();
        }
    };

    // Verify state matches
    if auth_state.state != returned_state {
        tracing::warn!(event = "state_mismatch");
        return (StatusCode::BAD_REQUEST, "State mismatch").into_response();
    }

    let login = match state.consume_login(&auth_state.state).await {
        Some(login) => login,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "Unknown, expired or consumed login state",
            )
                .into_response()
        }
    };
    if params.error.is_some() {
        tracing::warn!(event = "login_error");
        return (StatusCode::BAD_REQUEST, "Login failed").into_response();
    }
    let code = match params.code {
        Some(code) if !code.is_empty() && code.len() <= 16 * 1024 => code,
        _ => return (StatusCode::BAD_REQUEST, "Invalid code parameter").into_response(),
    };

    // Exchange code + verifier for tokens
    let token_resp = match exchange_code(
        &state.http,
        &state.token_endpoint().await,
        &state.cfg.client_id,
        &code,
        &auth_state.verifier,
        &state.cfg.redirect_uri,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(event = "token_exchange_failed", error = e);
            return (StatusCode::BAD_GATEWAY, "Token exchange failed").into_response();
        }
    };

    let Some(id_token) = token_resp.id_token.as_deref() else {
        return (StatusCode::BAD_GATEWAY, "Missing login identity").into_response();
    };
    let identity = match super::id_token::verify(
        id_token,
        &login.nonce,
        &token_resp.access_token,
        state,
    )
    .await
    {
        Ok(identity) => identity,
        Err(_) => return (StatusCode::BAD_GATEWAY, "Login identity rejected").into_response(),
    };
    let access =
        match super::layer::validate_access_token_str(&token_resp.access_token, state).await {
            Ok(claims) if Some(claims.sub.as_str()) == identity["sub"].as_str() => claims,
            _ => return (StatusCode::BAD_GATEWAY, "Login access token rejected").into_response(),
        };

    if !login.is_live() {
        return (
            StatusCode::BAD_REQUEST,
            "Login state expired during verification",
        )
            .into_response();
    }
    let (cookie_value, max_age) = match state.issue_session(token_resp, access, identity).await {
        Ok(session) => session,
        Err(e) => {
            tracing::warn!(event = "session_issue_failed", error = %e);
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };

    if !login.is_live() {
        state.revoke(&cookie_value).await;
        return (
            StatusCode::BAD_REQUEST,
            "Login state expired during issuance",
        )
            .into_response();
    }
    let session_cookie = super::session_cookie_header(&state.cfg, &cookie_value, max_age);

    let return_to = if auth_state.return_to.is_empty() || auth_state.return_to == "/" {
        "/".to_string()
    } else {
        auth_state.return_to
    };

    tracing::info!(event = "login");

    let mut resp = (StatusCode::FOUND, "").into_response();
    let headers = resp.headers_mut();
    headers.append(
        header::LOCATION,
        return_to.parse().unwrap_or_else(|_| "/".parse().unwrap()),
    );
    headers.append(
        header::SET_COOKIE,
        session_cookie.parse().expect("valid cookie header"),
    );
    resp
}

// ── Logout handler ───────────────────────────────────────────────────────────

pub async fn handle_logout(
    State(state): State<Arc<BrowserLayerState>>,
    headers: cli_framework::axum::http::HeaderMap,
) -> Response {
    if !super::browser_origin_allowed(&headers, &state.cfg) {
        return (StatusCode::FORBIDDEN, "Browser origin rejected").into_response();
    }
    let existed = match extract_cookie_value(&headers, &state.cfg.cookie_name) {
        Some(cookie) => state.revoke(cookie).await,
        None => false,
    };
    tracing::info!(event = "logout");

    let mut redirect_target = "/".to_string();
    if existed {
        if let Some(endpoint) = state.end_session_endpoint().await {
            if let Ok(mut url) = url::Url::parse(&endpoint) {
                if !url.query_pairs().any(|(name, _)| {
                    matches!(
                        name.as_ref(),
                        "post_logout_redirect_uri" | "id_token_hint" | "client_id" | "state"
                    )
                }) {
                    let root = format!(
                        "{}/",
                        url::Url::parse(&state.cfg.redirect_uri)
                            .expect("validated callback")
                            .origin()
                            .ascii_serialization()
                    );
                    url.query_pairs_mut().extend_pairs([
                        ("post_logout_redirect_uri", root.as_str()),
                        ("client_id", state.cfg.client_id.as_str()),
                    ]);
                    redirect_target = url.into();
                }
            }
        }
    }

    let secure_flag = super::secure_cookie_suffix(&state.cfg);

    let clear_session = format!(
        "{}=; Max-Age=0; HttpOnly{}; SameSite=Lax; Path=/",
        state.cfg.cookie_name, secure_flag
    );

    let mut resp = (StatusCode::FOUND, "").into_response();
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    h.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    h.append(
        header::LOCATION,
        redirect_target
            .parse()
            .unwrap_or_else(|_| "/".parse().unwrap()),
    );
    h.append(
        header::SET_COOKIE,
        clear_session.parse().expect("valid cookie header"),
    );
    resp
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn clear_cookie(name: &str, path: &str, cfg: &super::OidcBrowserSessionConfig) -> String {
    format!(
        "{name}=; Max-Age=0; HttpOnly{}; SameSite=Lax; Path={path}",
        super::secure_cookie_suffix(cfg)
    )
}

// ── Token exchange ───────────────────────────────────────────────────────────

pub(crate) struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub refresh_expires_in: u64,
    pub id_token: Option<String>,
}

impl TokenResponse {
    pub(crate) fn refresh_expires_at(&self) -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_add(self.refresh_expires_in)
            .min(i64::MAX as u64) as i64
    }
}

async fn exchange_code(
    http: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, String> {
    crate::endpoint_security::secure_endpoint(token_endpoint)
        .map_err(|_| "invalid token endpoint")?;
    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect_uri),
    ];

    let resp = http
        .post(token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|_| "token request failed")?;

    let body = crate::jwks::bounded_json(resp, 64 * 1024).await?;
    parse_token_response(&body, "")
}

pub(crate) async fn refresh_tokens(
    http: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse, String> {
    crate::endpoint_security::secure_endpoint(token_endpoint)
        .map_err(|_| "invalid token endpoint")?;
    let params = [
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_token),
    ];

    let resp = http
        .post(token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|_| "token request failed")?;

    let body = crate::jwks::bounded_json(resp, 64 * 1024).await?;
    parse_token_response(&body, refresh_token)
}

fn parse_token_response(
    body: &serde_json::Value,
    previous_refresh: &str,
) -> Result<TokenResponse, String> {
    if !body["token_type"]
        .as_str()
        .is_some_and(|value| value.eq_ignore_ascii_case("Bearer"))
    {
        return Err("unsupported or missing token type".into());
    }
    let refresh_expires_in = match body.get("refresh_expires_in") {
        None => 1800,
        Some(value) => value
            .as_u64()
            .filter(|value| *value <= i64::MAX as u64 / 2)
            .ok_or("invalid refresh-token lifetime")?,
    };
    let id_token = match body.get("id_token") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or("invalid ID token field")?
                .to_string(),
        ),
    };
    let refresh_token = match body.get("refresh_token") {
        None => previous_refresh.to_string(),
        Some(value) => value
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or("invalid refresh token field")?
            .to_string(),
    };
    Ok(TokenResponse {
        id_token,
        access_token: body["access_token"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or("missing access_token")?
            .to_string(),
        refresh_token,
        refresh_expires_in,
    })
}
