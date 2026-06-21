/// Shared JWKS fetching and caching logic used by both `server` and `browser` features.
use jsonwebtoken::DecodingKey;
use serde_json::Value as JsonValue;
use std::time::{Duration, Instant};

// ── Cache types ──────────────────────────────────────────────────────────────

pub(crate) struct JwksCache {
    pub keys: Vec<(Option<String>, DecodingKey)>, // (kid, key)
    pub fetched_at: Option<Instant>,
}

impl JwksCache {
    pub fn empty() -> Self {
        Self {
            keys: vec![],
            fetched_at: None,
        }
    }

    pub fn is_fresh(&self, ttl: Duration) -> bool {
        self.fetched_at.is_some_and(|t| t.elapsed() < ttl)
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// OIDC discovery document — extended to include fields needed by the browser feature.
pub(crate) struct OidcDiscovery {
    pub jwks_uri: String,
    /// Token endpoint for token exchange and refresh (browser feature).
    #[allow(dead_code)]
    pub token_endpoint: String,
    /// End-session endpoint for logout (browser feature; optional).
    #[allow(dead_code)]
    pub end_session_endpoint: Option<String>,
}

pub(crate) enum KeyResult {
    Keys(Vec<DecodingKey>),
    Unavailable,
    UnknownKid,
}

// ── Key filtering ────────────────────────────────────────────────────────────

pub(crate) fn filter_keys(
    all: &[(Option<String>, DecodingKey)],
    kid: &Option<String>,
) -> KeyResult {
    if all.is_empty() {
        return KeyResult::Unavailable;
    }
    let matching: Vec<DecodingKey> = match kid {
        Some(k) => all
            .iter()
            .filter(|(id, _)| id.as_deref() == Some(k.as_str()))
            .map(|(_, key)| key.clone())
            .collect(),
        None => {
            if all.len() == 1 {
                all.iter().map(|(_, key)| key.clone()).collect()
            } else {
                return KeyResult::UnknownKid;
            }
        }
    };
    if matching.is_empty() && kid.is_some() {
        KeyResult::UnknownKid
    } else if matching.is_empty() {
        KeyResult::Unavailable
    } else {
        KeyResult::Keys(matching)
    }
}

// ── Network fetching ─────────────────────────────────────────────────────────

pub(crate) async fn fetch_discovery(
    issuer_url: &str,
    http: &reqwest::Client,
) -> Result<OidcDiscovery, String> {
    let url = format!("{}/.well-known/openid-configuration", issuer_url);
    let resp = http.get(&url).send().await.map_err(|e| e.to_string())?;
    let doc: JsonValue = resp.json().await.map_err(|e| e.to_string())?;

    // Verify the discovery doc's issuer matches the configured issuer_url.
    let discovered_issuer = doc["issuer"]
        .as_str()
        .ok_or_else(|| "missing issuer in discovery doc".to_string())?;
    let normalized_configured = crate::normalize_issuer(issuer_url).map_err(|e| e.to_string())?;
    let normalized_discovered =
        crate::normalize_issuer(discovered_issuer).map_err(|e| e.to_string())?;
    if normalized_configured != normalized_discovered {
        return Err(format!(
            "discovery issuer mismatch: expected {normalized_configured}, got {normalized_discovered}"
        ));
    }

    let jwks_uri = doc["jwks_uri"]
        .as_str()
        .ok_or_else(|| "missing jwks_uri in discovery doc".to_string())?
        .to_string();
    crate::validate_jwks_uri(&jwks_uri).map_err(|e| e.to_string())?;

    let token_endpoint = doc["token_endpoint"]
        .as_str()
        .ok_or_else(|| "missing token_endpoint in discovery doc".to_string())?
        .to_string();

    let end_session_endpoint = doc["end_session_endpoint"].as_str().map(String::from);

    Ok(OidcDiscovery {
        jwks_uri,
        token_endpoint,
        end_session_endpoint,
    })
}

pub(crate) async fn fetch_jwks(
    jwks_uri: &str,
    http: &reqwest::Client,
) -> Result<Vec<(Option<String>, DecodingKey)>, String> {
    let resp = http.get(jwks_uri).send().await.map_err(|e| e.to_string())?;
    let doc: JsonValue = resp.json().await.map_err(|e| e.to_string())?;

    let keys_arr = doc["keys"].as_array().ok_or("missing keys array")?;
    let mut result = vec![];

    for jwk in keys_arr {
        let kid = jwk["kid"].as_str().map(String::from);
        let kty = jwk["kty"].as_str().unwrap_or("");

        let key = match kty {
            "RSA" => {
                let n = jwk["n"].as_str().unwrap_or("");
                let e = jwk["e"].as_str().unwrap_or("");
                DecodingKey::from_rsa_components(n, e).map_err(|e| e.to_string())?
            }
            "EC" => {
                let x = jwk["x"].as_str().unwrap_or("");
                let y = jwk["y"].as_str().unwrap_or("");
                DecodingKey::from_ec_components(x, y).map_err(|e| e.to_string())?
            }
            _ => continue,
        };

        result.push((kid, key));
    }

    Ok(result)
}

pub(crate) fn map_jwt_error(e: &jsonwebtoken::errors::Error) -> String {
    use jsonwebtoken::errors::ErrorKind;
    match e.kind() {
        ErrorKind::ExpiredSignature => "expired".to_string(),
        ErrorKind::ImmatureSignature => "not_yet_valid".to_string(),
        ErrorKind::InvalidSignature => "invalid_signature".to_string(),
        ErrorKind::InvalidIssuer => "invalid_issuer".to_string(),
        ErrorKind::InvalidAudience => "invalid_audience".to_string(),
        ErrorKind::InvalidAlgorithm => "unsupported_algorithm".to_string(),
        _ => "malformed_token".to_string(),
    }
}
