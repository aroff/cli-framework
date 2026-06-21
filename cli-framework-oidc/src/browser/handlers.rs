/// Axum handlers for /callback and /logout routes.
use super::auth_state::{decode_auth_state, AuthState};
use super::cookie::encrypt_cookie;
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
    pub error_description: Option<String>,
}

pub async fn handle_callback(
    State(state): State<Arc<BrowserLayerState>>,
    Query(params): Query<CallbackParams>,
    headers: cli_framework::axum::http::HeaderMap,
) -> Response {
    // Keycloak returned an error
    if let Some(ref err) = params.error {
        let desc = params
            .error_description
            .as_deref()
            .unwrap_or("authorization failed");
        tracing::warn!(event = "login_error", error = err, description = desc);
        return (
            StatusCode::BAD_REQUEST,
            format!("Login failed: {err}: {desc}"),
        )
            .into_response();
    }

    let code = match params.code {
        Some(ref c) => c.clone(),
        None => {
            return (StatusCode::BAD_REQUEST, "Missing code parameter").into_response();
        }
    };

    let returned_state = match params.state {
        Some(ref s) => s.clone(),
        None => {
            return (StatusCode::BAD_REQUEST, "Missing state parameter").into_response();
        }
    };

    // Read and verify __auth_state cookie
    let auth_state_value = match extract_cookie_value(&headers, "__auth_state") {
        Some(v) => v,
        None => {
            return (StatusCode::BAD_REQUEST, "Missing auth state cookie").into_response();
        }
    };

    let auth_state: AuthState = match decode_auth_state(auth_state_value, &state.hmac_key) {
        Some(s) => s,
        None => {
            tracing::warn!(event = "auth_state_invalid");
            let mut resp =
                (StatusCode::BAD_REQUEST, "Invalid or expired auth state").into_response();
            // Clear the bad __auth_state cookie
            resp.headers_mut().append(
                header::SET_COOKIE,
                clear_cookie("__auth_state", "/callback")
                    .parse()
                    .expect("valid header"),
            );
            return resp;
        }
    };

    // Verify state matches
    if auth_state.state != returned_state {
        tracing::warn!(event = "state_mismatch");
        let mut resp = (StatusCode::BAD_REQUEST, "State mismatch").into_response();
        resp.headers_mut().append(
            header::SET_COOKIE,
            clear_cookie("__auth_state", "/callback")
                .parse()
                .expect("valid header"),
        );
        return resp;
    }

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

    // Build and seal the session cookie
    let refresh_exp = token_resp.refresh_expires_at();
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

    let is_loopback = state.cfg.redirect_uri.starts_with("http://127.0.0.1")
        || state.cfg.redirect_uri.starts_with("http://localhost");
    let secure_flag = if is_loopback { "" } else { "; Secure" };

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
    // Clear the auth state cookie
    headers.append(
        header::SET_COOKIE,
        clear_cookie("__auth_state", "/callback")
            .parse()
            .expect("valid header"),
    );
    resp
}

// ── Logout handler ───────────────────────────────────────────────────────────

pub async fn handle_logout(
    State(state): State<Arc<BrowserLayerState>>,
    headers: cli_framework::axum::http::HeaderMap,
) -> Response {
    // Read sub from session cookie if present (for logging only)
    let sub = extract_cookie_value(&headers, &state.cfg.cookie_name)
        .and_then(|v| super::cookie::decrypt_cookie(state.cfg.session_key.as_bytes(), v).ok())
        .and_then(|payload| {
            // Try to extract sub from the access token without full validation
            extract_sub_from_jwt(&payload.access_token)
        });

    tracing::info!(event = "logout", sub = sub.as_deref().unwrap_or("unknown"));

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

    let is_loopback = state.cfg.redirect_uri.starts_with("http://127.0.0.1")
        || state.cfg.redirect_uri.starts_with("http://localhost");
    let secure_flag = if is_loopback { "" } else { "; Secure" };

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

fn extract_cookie_value<'a>(
    headers: &'a cli_framework::axum::http::HeaderMap,
    name: &str,
) -> Option<&'a str> {
    let header_val = headers.get(header::COOKIE)?;
    let s = header_val.to_str().ok()?;
    for pair in s.split(';') {
        let pair = pair.trim();
        if let Some(rest) = pair.strip_prefix(name) {
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value);
            }
        }
    }
    None
}

fn clear_cookie(name: &str, path: &str) -> String {
    format!("{name}=; Max-Age=0; HttpOnly; Secure; SameSite=Lax; Path={path}")
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

fn extract_sub_from_jwt(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() < 2 {
        return None;
    }
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let payload = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    v["sub"].as_str().map(String::from)
}

// ── Token exchange ───────────────────────────────────────────────────────────

pub(crate) struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub refresh_expires_in: u64,
}

impl TokenResponse {
    fn refresh_expires_at(&self) -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        (now + self.refresh_expires_in) as i64
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
        .map_err(|e| e.to_string())?;

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    if !status.is_success() {
        let err = body["error"].as_str().unwrap_or("unknown");
        let desc = body["error_description"].as_str().unwrap_or("");
        return Err(format!("{err}: {desc}"));
    }

    Ok(TokenResponse {
        access_token: body["access_token"]
            .as_str()
            .ok_or("missing access_token")?
            .to_string(),
        refresh_token: body["refresh_token"]
            .as_str()
            .ok_or("missing refresh_token")?
            .to_string(),
        refresh_expires_in: body["refresh_expires_in"].as_u64().unwrap_or(1800),
    })
}

pub(crate) async fn refresh_tokens(
    http: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse, String> {
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
        .map_err(|e| e.to_string())?;

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    if !status.is_success() {
        let err = body["error"].as_str().unwrap_or("unknown");
        let desc = body["error_description"].as_str().unwrap_or("");
        return Err(format!("{err}: {desc}"));
    }

    Ok(TokenResponse {
        access_token: body["access_token"]
            .as_str()
            .ok_or("missing access_token")?
            .to_string(),
        refresh_token: body["refresh_token"]
            .as_str()
            .unwrap_or(refresh_token) // some Keycloak configs don't rotate
            .to_string(),
        refresh_expires_in: body["refresh_expires_in"].as_u64().unwrap_or(1800),
    })
}
