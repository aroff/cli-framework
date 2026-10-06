//! Local, bounded browser credentials and revocable request/stream access.
use super::{
    handlers::{refresh_tokens, TokenResponse},
    layer::validate_access_token_str,
    state::BrowserLayerState,
    OidcClaims,
};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{watch, Mutex};
use zeroize::Zeroizing;

const MAX_SESSIONS: usize = 1024;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BrowserSessionError {
    #[error("browser session is unknown or malformed")]
    Invalid,
    #[error("browser session has expired or been revoked")]
    Expired,
    #[error("browser identity service is unavailable")]
    Unavailable,
    #[error("browser session capacity is exhausted")]
    Capacity,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

pub(crate) struct SessionRecord {
    deadline: Instant,
    expires_at: i64,
    refresh_exp: AtomicI64,
    live: watch::Sender<bool>,
    tokens: Mutex<SessionTokens>,
}

struct SessionTokens {
    access_token: Zeroizing<String>,
    refresh_token: Zeroizing<String>,
    claims: OidcClaims,
    identity: serde_json::Value,
    // Set before possible provider handoff, including cancellation. Never blindly
    // replay an operation that could have rotated the refresh token.
    refresh_uncertain: bool,
    next_refresh: Instant,
}

impl SessionRecord {
    pub fn revoke(&self) {
        self.live.send_replace(false);
    }

    fn is_live(&self) -> bool {
        let live = *self.live.borrow()
            && Instant::now() < self.deadline
            && now() < self.expires_at
            && now() < self.refresh_exp.load(Ordering::Acquire);
        if !live {
            self.revoke();
        }
        live
    }

    async fn invalidated(&self, token_exp: Option<i64>) {
        let mut changes = self.live.subscribe();
        loop {
            if !self.is_live() || token_exp.is_some_and(|expiry| now() >= expiry) {
                return;
            }
            let expiry = self
                .expires_at
                .min(self.refresh_exp.load(Ordering::Acquire))
                .min(token_exp.unwrap_or(i64::MAX));
            let remaining = Duration::from_secs(expiry.saturating_sub(now()).max(0) as u64);
            tokio::select! {
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(self.deadline)) => return,
                _ = tokio::time::sleep(remaining.min(Duration::from_secs(1))) => {},
                changed = changes.changed() => { if changed.is_err() { return; } },
            }
        }
    }
}

/// A verified request identity bound to local session revocation and token expiry.
/// Hosts can retain this for streams and recheck it before admitting mutations.
#[derive(Clone)]
pub struct BrowserSessionAccess {
    record: Arc<SessionRecord>,
    claims: OidcClaims,
}

impl BrowserSessionAccess {
    pub fn claims(&self) -> &OidcClaims {
        &self.claims
    }
    pub fn is_live(&self) -> bool {
        self.record.is_live() && now() < self.claims.exp
    }
    pub async fn invalidated(&self) {
        self.record.invalidated(Some(self.claims.exp)).await;
    }
    pub(crate) fn cookie_max_age(&self) -> u64 {
        self.record
            .expires_at
            .min(self.record.refresh_exp.load(Ordering::Acquire))
            .saturating_sub(now())
            .max(0) as u64
    }
}

fn valid_id(cookie: &str) -> bool {
    cookie.len() == 46
        && cookie.starts_with("s2.")
        && cookie[3..]
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_'))
}

impl BrowserLayerState {
    pub async fn issue_session(
        &self,
        tokens: TokenResponse,
        claims: OidcClaims,
        identity: serde_json::Value,
    ) -> Result<(String, u64), BrowserSessionError> {
        let current = now();
        let ttl_secs = self
            .cfg
            .session_ttl
            .as_secs()
            .saturating_add(u64::from(self.cfg.session_ttl.subsec_nanos() != 0));
        let expires_at = current
            .checked_add(i64::try_from(ttl_secs).map_err(|_| BrowserSessionError::Invalid)?)
            .ok_or(BrowserSessionError::Invalid)?;
        let deadline = Instant::now()
            .checked_add(self.cfg.session_ttl)
            .ok_or(BrowserSessionError::Invalid)?;
        let refresh_exp = if tokens.refresh_token.is_empty() {
            claims.exp
        } else {
            tokens.refresh_expires_at()
        };
        if current >= refresh_exp || current >= claims.exp {
            return Err(BrowserSessionError::Expired);
        }
        let cookie = format!("s2.{}", super::auth_state::random_state());
        let (live, _) = watch::channel(true);
        let record = Arc::new(SessionRecord {
            deadline,
            expires_at,
            refresh_exp: AtomicI64::new(refresh_exp),
            live,
            tokens: Mutex::new(SessionTokens {
                access_token: Zeroizing::new(tokens.access_token),
                refresh_token: Zeroizing::new(tokens.refresh_token),
                claims,
                identity,
                refresh_uncertain: false,
                next_refresh: Instant::now(),
            }),
        });
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, record| record.is_live());
        if sessions.len() >= MAX_SESSIONS {
            return Err(BrowserSessionError::Capacity);
        }
        if sessions.contains_key(&cookie) {
            return Err(BrowserSessionError::Invalid);
        }
        sessions.insert(cookie.clone(), record);
        Ok((
            cookie,
            expires_at.min(refresh_exp).saturating_sub(current).max(0) as u64,
        ))
    }

    pub async fn revoke(&self, cookie: &str) -> bool {
        if !valid_id(cookie) {
            return false;
        }
        if let Some(record) = self.sessions.lock().await.remove(cookie) {
            record.revoke();
            return true;
        }
        false
    }

    pub async fn authenticate(
        &self,
        cookie: &str,
    ) -> Result<BrowserSessionAccess, BrowserSessionError> {
        if !valid_id(cookie) {
            return Err(BrowserSessionError::Invalid);
        }
        let record = self
            .sessions
            .lock()
            .await
            .get(cookie)
            .cloned()
            .ok_or(BrowserSessionError::Invalid)?;
        if !record.is_live() {
            return Err(BrowserSessionError::Expired);
        }
        let mut tokens = tokio::select! {
            _ = record.invalidated(None) => return Err(BrowserSessionError::Expired),
            tokens = record.tokens.lock() => tokens,
        };
        if !record.is_live() {
            return Err(BrowserSessionError::Expired);
        }
        let needs_refresh = tokens.claims.exp
            <= now().saturating_add(self.cfg.refresh_skew.as_secs().min(i64::MAX as u64) as i64);
        let refresh_due = tokens.claims.exp <= now() || Instant::now() >= tokens.next_refresh;
        if needs_refresh
            && refresh_due
            && !tokens.refresh_token.is_empty()
            && !tokens.refresh_uncertain
        {
            tokens.refresh_uncertain = true;
            let endpoint = self.token_endpoint().await;
            let response = tokio::select! {
                _ = record.invalidated(None) => return Err(BrowserSessionError::Expired),
                response = refresh_tokens(&self.http, &endpoint, &self.cfg.client_id, &tokens.refresh_token) => response,
            };
            if let Ok(response) = response {
                if let Ok(claims) = validate_access_token_str(&response.access_token, self).await {
                    let identity_valid = match response.id_token.as_deref() {
                        None => true,
                        Some(identity) => super::id_token::verify_refresh(
                            identity,
                            &tokens.identity,
                            &response.access_token,
                            self,
                        )
                        .await
                        .is_ok(),
                    };
                    if identity_valid
                        && claims.sub == tokens.claims.sub
                        && claims.exp > now()
                        && record.is_live()
                    {
                        let refresh_exp = response.refresh_expires_at();
                        if refresh_exp <= now() {
                            record.revoke();
                            return Err(BrowserSessionError::Expired);
                        }
                        tokens.access_token = Zeroizing::new(response.access_token);
                        tokens.refresh_token = Zeroizing::new(response.refresh_token);
                        tokens.claims = claims;
                        tokens.refresh_uncertain = false;
                        // Short-lived provider tokens can remain inside the skew
                        // window after refresh. Give the new token half its
                        // remaining lifetime before proactive refresh, so queued
                        // requests don't each rotate the same session again.
                        let remaining = tokens.claims.exp.saturating_sub(now()).max(0) as u64;
                        let refreshed_at = Instant::now();
                        let cooldown = (Duration::from_secs(remaining) / 2)
                            .min(self.cfg.refresh_skew)
                            .min(record.deadline.saturating_duration_since(refreshed_at));
                        tokens.next_refresh = refreshed_at + cooldown;
                        record.refresh_exp.store(refresh_exp, Ordering::Release);
                        // Notify lifetime waiters without ever resetting revocation.
                        record.live.send_modify(|_| {});
                    }
                }
            }
        }
        if !record.is_live() || tokens.claims.exp <= now() {
            return Err(BrowserSessionError::Expired);
        }
        let claims = validate_access_token_str(&tokens.access_token, self)
            .await
            .map_err(|_| BrowserSessionError::Unavailable)?;
        if !record.is_live() || claims.exp <= now() {
            return Err(BrowserSessionError::Expired);
        }
        Ok(BrowserSessionAccess {
            record: record.clone(),
            claims,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{
        AudiencePolicy, OidcBrowserSession, OidcBrowserSessionConfig, SessionKey,
    };

    fn runtime(ttl: Duration) -> OidcBrowserSession {
        let mut config = OidcBrowserSessionConfig::new(
            "https://issuer.example",
            "client",
            "https://app.example/callback",
            SessionKey::from_bytes([7; 32]),
            AudiencePolicy::Unchecked,
        );
        config.session_ttl = ttl;
        OidcBrowserSession::new(config).unwrap()
    }

    async fn issue(session: &OidcBrowserSession) -> Result<(String, u64), BrowserSessionError> {
        let claims = OidcClaims {
            iss: "https://issuer.example".into(),
            sub: "owner".into(),
            aud: vec![],
            exp: now() + 300,
            iat: None,
            nbf: None,
            preferred_username: None,
            email: None,
            scopes: vec![],
            roles: vec![],
            groups: vec![],
            raw: serde_json::json!({}),
        };
        session
            .state
            .issue_session(
                TokenResponse {
                    access_token: "verified-access-fixture".into(),
                    refresh_token: "refresh".into(),
                    refresh_expires_in: 900,
                    id_token: None,
                },
                claims,
                serde_json::json!({"sub": "owner"}),
            )
            .await
    }

    #[tokio::test]
    async fn capacity_is_bounded_and_revocation_releases_a_slot() {
        let session = runtime(Duration::from_secs(300));
        let (first, _) = issue(&session).await.unwrap();
        for _ in 1..MAX_SESSIONS {
            issue(&session).await.unwrap();
        }
        assert!(matches!(
            issue(&session).await,
            Err(BrowserSessionError::Capacity)
        ));
        session.revoke_cookie(&first).await;
        issue(&session).await.unwrap();
        assert_eq!(session.state.sessions.lock().await.len(), MAX_SESSIONS);
    }

    #[tokio::test]
    async fn expiry_reclaims_records_and_same_key_restart_rejects_old_cookie() {
        let session = runtime(Duration::from_millis(20));
        let (old, _) = issue(&session).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        issue(&session).await.unwrap();
        assert_eq!(session.state.sessions.lock().await.len(), 1);
        assert!(!session.state.sessions.lock().await.contains_key(&old));
        let restarted = runtime(Duration::from_secs(300));
        assert!(matches!(
            restarted.authenticate_cookie(&old).await,
            Err(BrowserSessionError::Invalid)
        ));
    }
}
