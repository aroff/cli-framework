//! `{prefix}/login`, `/callback`, `/logout` and `/session`.

use super::seal::Purpose;
use super::{random_token, read_cookie, EndReason, Inner, Resolution, TokenCallError};
use crate::browser::pkce::{derive_challenge, generate_verifier};
use crate::browser::request_type::validate_return_to;
use cli_framework::axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as JsonValue};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) fn router(inner: Arc<Inner>) -> Router {
    let p = inner.cfg.route_prefix.clone();
    Router::new()
        .route(&format!("{p}/login"), get(login))
        .route(&format!("{p}/callback"), get(callback))
        .route(&format!("{p}/logout"), post(logout))
        .route(&format!("{p}/session"), get(session))
        .with_state(inner)
}

/// What the sign-in state cookie holds between `/login` and `/callback`.
#[derive(Serialize, Deserialize)]
struct SignIn {
    /// `state` sent to the realm.
    s: String,
    /// PKCE verifier.
    v: String,
    /// ID-token nonce.
    n: String,
    /// Where to go after sign-in.
    r: String,
    /// Unix seconds when sign-in started.
    t: i64,
}

fn no_store(mut resp: Response) -> Response {
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

fn redirect(status: StatusCode, location: &str) -> Response {
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = status;
    if let Ok(v) = HeaderValue::from_str(location) {
        resp.headers_mut().insert(header::LOCATION, v);
    }
    no_store(resp)
}

fn plain(status: StatusCode, text: &'static str) -> Response {
    no_store(
        (
            status,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            text,
        )
            .into_response(),
    )
}

fn append_cookie(resp: &mut Response, value: HeaderValue) {
    resp.headers_mut().append(header::SET_COOKIE, value);
}

impl Inner {
    fn sign_in_cookie(&self, value: &str, max_age: u64) -> HeaderValue {
        HeaderValue::from_str(&format!(
            "{}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age}",
            self.cfg.sign_in_cookie_name
        ))
        .expect("valid header value")
    }
}

async fn login(
    State(inner): State<Arc<Inner>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let return_to = match q.get("return_to").map(String::as_str) {
        None | Some("") => "/".to_string(),
        Some(r) => match validate_return_to(r) {
            Ok(r) => r,
            Err(_) => {
                return plain(
                    StatusCode::BAD_REQUEST,
                    "return_to must be a path on this site\n",
                )
            }
        },
    };
    let disc = match inner.discovery().await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("host-session: login: {e}");
            return plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "sign-in is unavailable; try again\n",
            );
        }
    };
    let sign_in = SignIn {
        s: random_token(),
        v: generate_verifier(),
        n: random_token(),
        r: return_to,
        t: inner.now(),
    };
    let mut scopes: Vec<&str> = vec!["openid"];
    scopes.extend(
        inner
            .cfg
            .scopes
            .iter()
            .map(String::as_str)
            .filter(|s| *s != "openid"),
    );
    let mut url = match url::Url::parse(&disc.authorization_endpoint) {
        Ok(u) => u,
        Err(_) => return plain(StatusCode::SERVICE_UNAVAILABLE, "sign-in is unavailable\n"),
    };
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &inner.cfg.client_id)
        .append_pair("redirect_uri", &inner.cfg.redirect_uri)
        .append_pair("scope", &scopes.join(" "))
        .append_pair("state", &sign_in.s)
        .append_pair("nonce", &sign_in.n)
        .append_pair("code_challenge", &derive_challenge(&sign_in.v))
        .append_pair("code_challenge_method", "S256");
    let sealed = inner.sealer.seal(
        Purpose::SignIn,
        &serde_json::to_vec(&sign_in).expect("serializable"),
    );
    let mut resp = redirect(StatusCode::FOUND, url.as_str());
    append_cookie(
        &mut resp,
        inner.sign_in_cookie(&sealed, inner.cfg.sign_in_ttl.as_secs()),
    );
    resp
}

async fn callback(
    State(inner): State<Arc<Inner>>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let clear_sign_in = inner.sign_in_cookie("", 0);
    let fail = |status: StatusCode, why: &str| {
        tracing::info!("host-session: sign-in refused: {why}");
        let mut resp = plain(status, "sign-in failed; start again\n");
        append_cookie(&mut resp, clear_sign_in.clone());
        resp
    };
    let Some(sign_in) = read_cookie(&headers, &inner.cfg.sign_in_cookie_name)
        .and_then(|v| inner.sealer.open(Purpose::SignIn, &v))
        .and_then(|b| serde_json::from_slice::<SignIn>(&b).ok())
    else {
        return fail(StatusCode::BAD_REQUEST, "no sign-in state cookie");
    };
    if inner.now() - sign_in.t > inner.cfg.sign_in_ttl.as_secs() as i64 {
        return fail(StatusCode::BAD_REQUEST, "sign-in state expired");
    }
    let state_ok = q
        .get("state")
        .is_some_and(|s| super::constant_time_eq(s.as_bytes(), sign_in.s.as_bytes()));
    if !state_ok {
        return fail(StatusCode::BAD_REQUEST, "state mismatch");
    }
    if let Some(err) = q.get("error") {
        return fail(StatusCode::BAD_REQUEST, &format!("realm error {err}"));
    }
    let Some(code) = q.get("code") else {
        return fail(StatusCode::BAD_REQUEST, "no code");
    };
    let tokens = match inner
        .token_call(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &inner.cfg.redirect_uri),
            ("code_verifier", &sign_in.v),
        ])
        .await
    {
        Ok(t) => t,
        Err(TokenCallError::Refused(e)) => {
            return fail(StatusCode::BAD_REQUEST, &format!("code refused: {e}"))
        }
        Err(TokenCallError::Unavailable(e)) => return fail(StatusCode::BAD_GATEWAY, &e),
    };
    let Some(id_token) = tokens.id_token.as_deref() else {
        return fail(StatusCode::BAD_GATEWAY, "no id_token");
    };
    let id = match inner.id_tokens.validate(id_token).await {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, &format!("id_token: {e}")),
    };
    let nonce_ok = id.raw["nonce"]
        .as_str()
        .is_some_and(|n| super::constant_time_eq(n.as_bytes(), sign_in.n.as_bytes()));
    if !nonce_ok {
        return fail(StatusCode::BAD_REQUEST, "id_token nonce mismatch");
    }
    let access = match inner.verified_access(&tokens.access_token).await {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, &format!("access_token: {e}")),
    };
    if access.sub != id.sub {
        return fail(
            StatusCode::BAD_GATEWAY,
            "access and id tokens name different subjects",
        );
    }
    let rec = match inner.new_record(tokens, access.exp) {
        Ok(r) => r,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, &e),
    };
    let cookie = match inner.session_cookie(&rec) {
        Ok(c) => c,
        Err(size) => {
            tracing::error!(
                size,
                max = inner.cfg.max_cookie_bytes,
                "host-session: the session cookie is over the browser limit"
            );
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "session cookie too large",
            );
        }
    };
    tracing::info!(cookie_bytes = cookie.len(), "host-session: signed in");
    let mut resp = redirect(StatusCode::SEE_OTHER, &sign_in.r);
    append_cookie(&mut resp, clear_sign_in);
    append_cookie(&mut resp, cookie);
    resp
}

/// Refuses a cross-site request: an `Origin` other than the host's own, or
/// `Sec-Fetch-Site: cross-site`.
pub(crate) fn cross_site(headers: &HeaderMap, origin: &str) -> bool {
    let bad_origin = headers
        .get(header::ORIGIN)
        .is_some_and(|o| o.as_bytes() != origin.as_bytes());
    let bad_site = headers
        .get("sec-fetch-site")
        .is_some_and(|s| s.as_bytes() == b"cross-site");
    bad_origin || bad_site
}

async fn logout(State(inner): State<Arc<Inner>>, headers: HeaderMap) -> Response {
    if cross_site(&headers, &inner.origin) {
        return plain(StatusCode::FORBIDDEN, "cross-site logout refused\n");
    }
    let refresh_token = read_cookie(&headers, &inner.cfg.cookie_name)
        .and_then(|v| inner.sealer.open(Purpose::Session, &v))
        .and_then(|b| super::seal::SessionRecord::decode(&b))
        .filter(|r| super::constant_time_eq(r.binding.as_bytes(), inner.cfg.binding.as_bytes()))
        .map(|r| r.refresh_token);
    let disc = inner.discovery().await.ok();
    let end_session = disc.and_then(|d| d.end_session_endpoint.clone());
    // End the realm's session from here, with the refresh token and the
    // client's credentials: the browser holds no ID token to hint with.
    if let (Some(endpoint), Some(rt)) = (&end_session, &refresh_token) {
        let mut body: Vec<(&str, &str)> =
            vec![("client_id", &inner.cfg.client_id), ("refresh_token", rt)];
        if let Some(secret) = &inner.cfg.client_secret {
            body.push(("client_secret", secret.expose()));
        }
        match inner.http.post(endpoint).form(&body).send().await {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => tracing::warn!("host-session: realm logout answered HTTP {}", r.status()),
            Err(e) => tracing::warn!("host-session: realm logout failed: {e}"),
        }
    }
    let location = match end_session.as_deref().map(url::Url::parse) {
        Some(Ok(mut u)) => {
            u.query_pairs_mut()
                .append_pair("client_id", &inner.cfg.client_id)
                .append_pair(
                    "post_logout_redirect_uri",
                    &inner.cfg.post_logout_redirect_uri,
                );
            u.to_string()
        }
        _ => inner.cfg.post_logout_redirect_uri.clone(),
    };
    let mut resp = redirect(StatusCode::SEE_OTHER, &location);
    append_cookie(&mut resp, inner.clear_cookie());
    resp
}

async fn session(State(inner): State<Arc<Inner>>, headers: HeaderMap) -> Response {
    match inner.resolve(&headers).await {
        Resolution::Active(s) => {
            let mut body = Map::new();
            body.insert("sub".into(), s.claims()["sub"].clone());
            for name in &inner.cfg.session_claims {
                if let Some(v) = s.claims().get(name) {
                    body.insert(name.clone(), v.clone());
                }
            }
            body.insert("exp".into(), json!(s.expires_at()));
            body.insert("idle_exp".into(), json!(s.idle_expires_at()));
            let mut resp = no_store(Json(JsonValue::Object(body)).into_response());
            if let Some(c) = s.set_cookie() {
                append_cookie(&mut resp, c.clone());
            }
            resp
        }
        Resolution::Ended {
            reason,
            clear_cookie,
        } => {
            let (status, error) = match reason {
                EndReason::Unavailable => {
                    (StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
                }
                _ => (StatusCode::UNAUTHORIZED, "unauthenticated"),
            };
            let mut resp = no_store((status, Json(json!({ "error": error }))).into_response());
            if let Some(c) = clear_cookie {
                append_cookie(&mut resp, c);
            }
            resp
        }
    }
}
