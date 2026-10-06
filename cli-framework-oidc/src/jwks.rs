/// Shared JWKS fetching and caching logic used by both `server` and `browser` features.
use crate::types::AudiencePolicy;
use jsonwebtoken::{DecodingKey, Validation};
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
    /// Browser login must use the advertised endpoint, never an issuer-specific path.
    #[allow(dead_code)]
    pub authorization_endpoint: Option<String>,
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

pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("cli-framework-oidc/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client")
}

/// Bound every response while reading, including chunked bodies without a
/// Content-Length. Provider errors and credentials never become error text.
pub(crate) async fn bounded_json(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<JsonValue, String> {
    if !response.status().is_success() {
        return Err("OIDC endpoint returned unsuccessful status".into());
    }
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err("OIDC response exceeds size limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "OIDC response read failed")?
    {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err("OIDC response exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "OIDC response is not valid JSON".into())
}

#[cfg(test)]
mod response_bounds_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn chunked_response_without_content_length_is_bounded() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let provider = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 256];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut chunk).await.unwrap();
                assert!(count > 0, "request ended before its headers");
                request.extend_from_slice(&chunk[..count]);
                assert!(
                    request.len() <= 4096,
                    "test request headers exceeded budget"
                );
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nC\r\nxxxxxxxxxxxx\r\nC\r\nxxxxxxxxxxxx\r\n0\r\n\r\n").await.unwrap();
        });
        let response = http_client()
            .get(format!("http://{address}/metadata"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.content_length(), None);
        assert_eq!(
            bounded_json(response, 16).await.unwrap_err(),
            "OIDC response exceeds size limit"
        );
        provider.await.unwrap();
    }
}

pub(crate) async fn fetch_discovery(
    issuer_url: &str,
    http: &reqwest::Client,
) -> Result<OidcDiscovery, String> {
    crate::endpoint_security::secure_issuer(issuer_url).map_err(|_| "invalid discovery issuer")?;
    let url = format!("{}/.well-known/openid-configuration", issuer_url);
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|_| "OIDC discovery request failed")?;
    let doc = bounded_json(resp, 1024 * 1024).await?;

    // Verify the discovery doc's issuer matches the configured issuer_url.
    let discovered_issuer = doc["issuer"]
        .as_str()
        .ok_or_else(|| "missing issuer in discovery doc".to_string())?;
    let normalized_configured =
        crate::normalize_issuer(issuer_url).map_err(|_| "invalid configured issuer")?;
    let normalized_discovered =
        crate::normalize_issuer(discovered_issuer).map_err(|_| "invalid discovered issuer")?;
    if normalized_configured != normalized_discovered {
        return Err("discovery issuer mismatch".into());
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
    crate::endpoint_security::secure_endpoint(&token_endpoint)
        .map_err(|_| "invalid token endpoint")?;

    let authorization_endpoint = doc["authorization_endpoint"].as_str().map(String::from);
    if let Some(endpoint) = &authorization_endpoint {
        crate::endpoint_security::secure_endpoint(endpoint)
            .map_err(|_| "invalid authorization endpoint")?;
    }

    let end_session_endpoint = doc["end_session_endpoint"].as_str().map(String::from);
    if let Some(endpoint) = &end_session_endpoint {
        crate::endpoint_security::secure_endpoint(endpoint)
            .map_err(|_| "invalid end-session endpoint")?;
    }

    Ok(OidcDiscovery {
        jwks_uri,
        authorization_endpoint,
        token_endpoint,
        end_session_endpoint,
    })
}

pub(crate) async fn fetch_jwks(
    jwks_uri: &str,
    http: &reqwest::Client,
) -> Result<Vec<(Option<String>, DecodingKey)>, String> {
    let resp = http
        .get(jwks_uri)
        .send()
        .await
        .map_err(|_| "JWKS request failed")?;
    let doc = bounded_json(resp, 1024 * 1024).await?;

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

/// Spec claims every validated token must carry, for
/// `Validation::set_required_spec_claims`. `jsonwebtoken` compares `iss` with
/// the configured issuer only when the claim is present, so without `"iss"`
/// here a token with no `iss` at all would pass. A missing `iss` is rejected
/// as an invalid issuer, a missing `exp` as a malformed token.
pub(crate) const REQUIRED_SPEC_CLAIMS: &[&str] = &["exp", "iss"];

/// `true` when `e` is a missing `iss` (see [`REQUIRED_SPEC_CLAIMS`]).
pub(crate) fn is_missing_iss(e: &jsonwebtoken::errors::Error) -> bool {
    matches!(e.kind(), jsonwebtoken::errors::ErrorKind::MissingRequiredClaim(c) if c == "iss")
}

/// Applies `policy` to `validation`. Under `Require` and `RequireAny`, `aud`
/// also becomes a required spec claim: `jsonwebtoken` compares `aud` with the
/// configured audience only when the claim is present, so without this a
/// token with no `aud` at all would pass. A missing `aud` is rejected as an
/// invalid audience (see [`is_missing_aud`]). Call it after
/// `set_required_spec_claims(REQUIRED_SPEC_CLAIMS)`, which replaces the set.
pub(crate) fn apply_audience_policy(validation: &mut Validation, policy: &AudiencePolicy) {
    match policy {
        AudiencePolicy::Require(aud) => validation.set_audience(&[aud]),
        AudiencePolicy::RequireAny(auds) => validation.set_audience(auds),
        AudiencePolicy::Unchecked => {
            validation.validate_aud = false;
            return;
        }
    }
    validation.required_spec_claims.insert("aud".to_string());
}

/// `true` when `e` is a missing `aud` (see [`apply_audience_policy`]).
pub(crate) fn is_missing_aud(e: &jsonwebtoken::errors::Error) -> bool {
    matches!(e.kind(), jsonwebtoken::errors::ErrorKind::MissingRequiredClaim(c) if c == "aud")
}

#[cfg(feature = "browser")]
pub(crate) fn map_jwt_error(e: &jsonwebtoken::errors::Error) -> String {
    use jsonwebtoken::errors::ErrorKind;
    if is_missing_iss(e) {
        return "invalid_issuer".to_string();
    }
    if is_missing_aud(e) {
        return "invalid_audience".to_string();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, encode, Algorithm, EncodingKey, Header};

    fn decode_without_iss() -> jsonwebtoken::errors::Error {
        let now = jsonwebtoken::get_current_timestamp();
        let token = encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({ "sub": "u", "exp": now + 300 }),
            &EncodingKey::from_secret(b"k"),
        )
        .expect("encode");
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&["https://issuer.test"]);
        validation.validate_aud = false;
        validation.set_required_spec_claims(REQUIRED_SPEC_CLAIMS);
        decode::<serde_json::Value>(&token, &DecodingKey::from_secret(b"k"), &validation)
            .expect_err("a token without iss must not validate")
    }

    #[test]
    fn missing_iss_is_rejected_and_recognised() {
        assert!(is_missing_iss(&decode_without_iss()));
    }

    fn decode_with(policy: &AudiencePolicy) -> jsonwebtoken::errors::Result<()> {
        let now = jsonwebtoken::get_current_timestamp();
        let token = encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({ "sub": "u", "iss": "https://issuer.test", "exp": now + 300 }),
            &EncodingKey::from_secret(b"k"),
        )
        .expect("encode");
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&["https://issuer.test"]);
        validation.set_required_spec_claims(REQUIRED_SPEC_CLAIMS);
        apply_audience_policy(&mut validation, policy);
        decode::<JsonValue>(&token, &DecodingKey::from_secret(b"k"), &validation).map(|_| ())
    }

    #[test]
    fn missing_aud_is_rejected_when_an_audience_is_required() {
        let err = decode_with(&AudiencePolicy::Require("api".into()))
            .expect_err("a token without aud must not validate under Require");
        assert!(is_missing_aud(&err));
        let err = decode_with(&AudiencePolicy::RequireAny(vec!["a".into(), "b".into()]))
            .expect_err("a token without aud must not validate under RequireAny");
        assert!(is_missing_aud(&err));
    }

    #[test]
    fn missing_aud_is_accepted_when_unchecked() {
        decode_with(&AudiencePolicy::Unchecked).expect("Unchecked ignores aud");
    }

    #[cfg(feature = "browser")]
    #[test]
    fn browser_maps_missing_iss_to_invalid_issuer() {
        assert_eq!(map_jwt_error(&decode_without_iss()), "invalid_issuer");
    }

    #[cfg(feature = "browser")]
    #[test]
    fn browser_maps_missing_aud_to_invalid_audience() {
        let err = decode_with(&AudiencePolicy::Require("api".into())).unwrap_err();
        assert_eq!(map_jwt_error(&err), "invalid_audience");
    }
}
