//! Synthesized-OIDC-issuer test helpers, promoted out of
//! `tests/server_validation.rs` (spec 021 testing decisions).
//!
//! These were private copies inside one test binary. This module is the
//! promotion of *the pattern*, not a second copy of it:
//! `tests/server_validation.rs` in this crate now calls these functions
//! directly (see that file), and `cli-framework`'s own `config-managed`
//! tests depend on this crate with `test-support` enabled to mint real
//! (test-signed) JWTs against a synthesized wiremock issuer.
//!
//! Keys are P-256 / ES256, generated via `rcgen` (backed by `ring`) — this
//! avoids the `rsa` crate, which carries RUSTSEC-2023-0071 with no upstream
//! fix, exactly as the original private copy did.
//!
//! `#[doc(hidden)]` because this is test-only surface, not a stable public
//! API this crate commits to: it exists to be depended on from `[dev-dependencies]`
//! (this crate's own tests, and downstream crates' tests), never from a real
//! application's runtime dependency graph.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::Algorithm;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::server::{AudiencePolicy, JwkSet, OidcValidationConfig};

/// A generated P-256 key pair usable both to mint a JWT (`encoding_key`) and
/// to publish the corresponding JWK (`x`/`y`/`kid`) — see [`jwk_for_key`].
#[doc(hidden)]
pub struct TestKeyPair {
    pub x: String,
    pub y: String,
    pub encoding_key: jsonwebtoken::EncodingKey,
    pub kid: String,
}

/// A fresh key pair with `kid = "test-kid-1"`.
#[doc(hidden)]
pub fn test_key_pair() -> TestKeyPair {
    test_key_pair_with_kid("test-kid-1")
}

/// A fresh key pair with an explicit `kid`.
#[doc(hidden)]
pub fn test_key_pair_with_kid(kid: &str) -> TestKeyPair {
    let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("key gen");
    // public_key_raw() returns the uncompressed EC point: 0x04 || x(32) || y(32)
    let point = kp.public_key_raw();
    assert_eq!(point.len(), 65, "P-256 uncompressed point must be 65 bytes");
    let x = URL_SAFE_NO_PAD.encode(&point[1..33]);
    let y = URL_SAFE_NO_PAD.encode(&point[33..65]);
    let encoding_key =
        jsonwebtoken::EncodingKey::from_ec_pem(kp.serialize_pem().as_bytes()).expect("enc key");
    TestKeyPair {
        x,
        y,
        encoding_key,
        kid: kid.to_string(),
    }
}

/// The JWK (with `kid`) for `kp`, suitable for a mocked `/jwks` response body.
#[doc(hidden)]
pub fn jwk_for_key(kp: &TestKeyPair) -> serde_json::Value {
    json!({
        "kty": "EC",
        "crv": "P-256",
        "kid": kp.kid,
        "alg": "ES256",
        "use": "sig",
        "x": kp.x,
        "y": kp.y,
    })
}

/// The JWK for `kp` with no `kid` field — for exercising the
/// no-kid/single-key and no-kid/multiple-keys resolution paths.
#[doc(hidden)]
pub fn jwk_for_key_no_kid(kp: &TestKeyPair) -> serde_json::Value {
    json!({
        "kty": "EC",
        "crv": "P-256",
        "alg": "ES256",
        "use": "sig",
        "x": kp.x,
        "y": kp.y,
    })
}

/// Mint a JWT for `claims`, signed with `kp` and stamping `kp.kid` in the header.
#[doc(hidden)]
pub fn mint_jwt(kp: &TestKeyPair, claims: serde_json::Value) -> String {
    let mut header = jsonwebtoken::Header::new(Algorithm::ES256);
    header.kid = Some(kp.kid.clone());
    jsonwebtoken::encode(&header, &claims, &kp.encoding_key).expect("encode")
}

/// Mint a JWT with no `kid` in the header at all.
#[doc(hidden)]
pub fn mint_jwt_no_kid(kp: &TestKeyPair, claims: serde_json::Value) -> String {
    let header = jsonwebtoken::Header::new(Algorithm::ES256);
    jsonwebtoken::encode(&header, &claims, &kp.encoding_key).expect("encode")
}

/// Mint a JWT signed with `kp` but stamping an explicit (possibly
/// mismatched) `kid` in the header — for wrong-key/unknown-kid tests.
#[doc(hidden)]
pub fn mint_jwt_with_kid(kp: &TestKeyPair, claims: serde_json::Value, kid: &str) -> String {
    let mut header = jsonwebtoken::Header::new(Algorithm::ES256);
    header.kid = Some(kid.to_string());
    jsonwebtoken::encode(&header, &claims, &kp.encoding_key).expect("encode")
}

/// Current Unix time in whole seconds, for building `exp`/`iat` claims.
#[doc(hidden)]
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// An [`OidcValidationConfig`] pointed at a `wiremock::MockServer` acting as
/// the synthesized issuer (`jwks_uri` set directly, bypassing discovery).
#[doc(hidden)]
pub fn make_cfg(issuer_uri: &str) -> OidcValidationConfig {
    OidcValidationConfig {
        jwks_uri: Some(format!("{issuer_uri}/jwks")),
        algorithms: vec![Algorithm::ES256],
        ..OidcValidationConfig::new(issuer_uri, AudiencePolicy::Unchecked)
    }
}

/// A synthesized issuer: an issuer URL, its own signing key, and its JWK set
/// given inline — no HTTP server, no network, no real identity provider.
/// Build several to exercise a multi-issuer validator (ADR 0082):
///
/// ```ignore
/// let a = TestIssuer::new("https://issuer-a.test");
/// let b = TestIssuer::new("https://issuer-b.test");
/// let validator = OidcValidator::new_multi([a.config(), b.config()])?;
/// let claims = validator.validate(&a.mint(json!({"sub": "u"}))).await?;
/// assert_eq!(claims.iss, "https://issuer-a.test");
/// ```
#[doc(hidden)]
pub struct TestIssuer {
    /// The issuer URL as given (use a normalized form, e.g. no trailing slash,
    /// so minted `iss` claims match the validator's normalized issuer).
    pub issuer: String,
    pub key: TestKeyPair,
}

impl TestIssuer {
    /// A new issuer with a fresh P-256 key whose `kid` is `"{issuer}#key-1"`,
    /// so two test issuers never share a `kid`.
    pub fn new(issuer: &str) -> Self {
        Self::with_kid(issuer, &format!("{issuer}#key-1"))
    }

    /// A new issuer with a fresh P-256 key and an explicit `kid`.
    pub fn with_kid(issuer: &str, kid: &str) -> Self {
        Self {
            issuer: issuer.to_string(),
            key: test_key_pair_with_kid(kid),
        }
    }

    /// This issuer's public JWK.
    pub fn jwk(&self) -> serde_json::Value {
        jwk_for_key(&self.key)
    }

    /// This issuer's JWK set, as served at a `jwks_uri` or passed inline.
    pub fn jwk_set(&self) -> JwkSet {
        serde_json::from_value(json!({ "keys": [self.jwk()] })).expect("valid JWK set")
    }

    /// A validation config trusting this issuer: inline keys
    /// ([`OidcValidationConfig::static_jwks`]), ES256, `AudiencePolicy::Unchecked`
    /// and default claim paths. Adjust fields before use.
    pub fn config(&self) -> OidcValidationConfig {
        OidcValidationConfig {
            algorithms: vec![Algorithm::ES256],
            ..OidcValidationConfig::new(&self.issuer, AudiencePolicy::Unchecked)
        }
        .with_static_jwks(self.jwk_set())
    }

    /// Mint a token signed by this issuer's key. `claims` must be a JSON
    /// object; `iss` (this issuer), `iat` (now) and `exp` (now + 300 s) are
    /// filled in when absent, so a caller may override any of them — e.g. set
    /// another issuer's `iss` to forge a cross-issuer token.
    pub fn mint(&self, claims: serde_json::Value) -> String {
        let mut claims = claims;
        let obj = claims
            .as_object_mut()
            .expect("claims must be a JSON object");
        let now = now_secs();
        obj.entry("iss").or_insert_with(|| json!(self.issuer));
        obj.entry("iat").or_insert_with(|| json!(now));
        obj.entry("exp").or_insert_with(|| json!(now + 300));
        mint_jwt(&self.key, claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_jwt_produces_three_segments() {
        let kp = test_key_pair();
        let token = mint_jwt(&kp, json!({"sub": "u", "exp": now_secs() + 60}));
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn jwk_for_key_carries_the_kid_jwk_for_key_no_kid_omits_it() {
        let kp = test_key_pair_with_kid("my-kid");
        let with_kid = jwk_for_key(&kp);
        assert_eq!(with_kid["kid"], "my-kid");
        let without_kid = jwk_for_key_no_kid(&kp);
        assert!(without_kid.get("kid").is_none());
    }

    #[test]
    fn mint_jwt_with_kid_overrides_header_kid() {
        let kp = test_key_pair_with_kid("original");
        let token = mint_jwt_with_kid(&kp, json!({"sub": "u"}), "overridden");
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.kid.as_deref(), Some("overridden"));
    }

    #[test]
    fn mint_jwt_no_kid_omits_header_kid() {
        let kp = test_key_pair();
        let token = mint_jwt_no_kid(&kp, json!({"sub": "u"}));
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert!(header.kid.is_none());
    }

    #[test]
    fn make_cfg_points_jwks_uri_at_issuer() {
        let cfg = make_cfg("https://issuer.example.com");
        assert_eq!(
            cfg.jwks_uri.as_deref(),
            Some("https://issuer.example.com/jwks")
        );
        assert_eq!(cfg.issuer_url, "https://issuer.example.com");
    }

    #[test]
    fn test_issuer_mints_with_defaults_and_publishes_its_key() {
        let issuer = TestIssuer::new("https://issuer-a.test");
        let token = issuer.mint(json!({"sub": "u"}));
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.kid.as_deref(), Some("https://issuer-a.test#key-1"));
        assert_eq!(issuer.jwk_set().keys.len(), 1);
        let cfg = issuer.config();
        assert!(cfg.static_jwks.is_some());
        assert!(cfg.jwks_uri.is_none());
        assert_eq!(cfg.issuer_url, "https://issuer-a.test");
    }

    #[test]
    fn now_secs_is_plausibly_recent() {
        assert!(now_secs() > 1_577_836_800);
    }
}
