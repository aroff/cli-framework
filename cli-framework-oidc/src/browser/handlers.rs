/// Axum handlers for /callback and /logout routes.
use super::auth_state::{decode_auth_state, AuthState};
use super::cookie::encrypt_cookie;
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
    let subject = match super::id_token::verify(
        id_token,
        &login.nonce,
        &token_resp.access_token,
        state,
    )
    .await
    {
        Ok(subject) => subject,
        Err(_) => return (StatusCode::BAD_GATEWAY, "Login identity rejected").into_response(),
    };
    let access =
        match super::layer::validate_access_token_str(&token_resp.access_token, state).await {
            Ok(claims) if claims.sub == subject => claims,
            _ => return (StatusCode::BAD_GATEWAY, "Login access token rejected").into_response(),
        };

    if !login.is_live() {
        return (
            StatusCode::BAD_REQUEST,
            "Login state expired during verification",
        )
            .into_response();
    }
    // Build and seal only verified matching identity/access tokens.
    let refresh_exp = if token_resp.refresh_token.is_empty() {
        access.exp
    } else {
        token_resp.refresh_expires_at()
    };
    let cookie_value = match encrypt_cookie(
        state.cfg.session_key.as_bytes(),
        &token_resp.access_token,
        &token_resp.refresh_token,
        refresh_exp,
    ) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(event = "cookie_encrypt_failed", error = %e);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let max_age = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let from_exp = (refresh_exp - now).max(0) as u64;
        from_exp.min(state.cfg.session_ttl.as_secs())
    };

    let secure_flag = super::secure_cookie_suffix(&state.cfg);

    let session_cookie = format!(
        "{}={}; HttpOnly{}; SameSite=Lax; Path=/; Max-Age={}",
        state.cfg.cookie_name, cookie_value, secure_flag, max_age
    );

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
    let _ = headers;
    tracing::info!(event = "logout");

    let end_session_url = state.end_session_endpoint().await;
    let app_root = {
        // Derive app root from redirect_uri (strip /callback)
        let uri = &state.cfg.redirect_uri;
        if let Some(pos) = uri.rfind('/') {
            uri[..pos].to_string()
        } else {
            uri.clone()
        }
    };

    let redirect_target = if let Some(ref url) = end_session_url {
        format!("{}?post_logout_redirect_uri={}", url, url_encode(&app_root))
    } else {
        "/".to_string()
    };

    let secure_flag = super::secure_cookie_suffix(&state.cfg);

    let clear_session = format!(
        "{}=; Max-Age=0; HttpOnly{}; SameSite=Lax; Path=/",
        state.cfg.cookie_name, secure_flag
    );

    let mut resp = (StatusCode::FOUND, "").into_response();
    let h = resp.headers_mut();
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

fn url_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                vec![c]
            }
            c => format!("%{:02X}", c as u32).chars().collect(),
        })
        .collect()
}

// ── Token exchange ───────────────────────────────────────────────────────────

pub(crate) struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub refresh_expires_in: u64,
    pub id_token: Option<String>,
}

impl TokenResponse {
    fn refresh_expires_at(&self) -> i64 {
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
    Ok(TokenResponse {
        id_token: body["id_token"].as_str().map(String::from),
        access_token: body["access_token"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or("missing access_token")?
            .to_string(),
        refresh_token: body["refresh_token"]
            .as_str()
            .unwrap_or(previous_refresh)
            .to_string(),
        refresh_expires_in,
    })
}
