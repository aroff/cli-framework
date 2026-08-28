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

use crate::server::{AudiencePolicy, OidcValidationConfig};

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
        issuer_url: issuer_uri.to_string(),
        audience: AudiencePolicy::Unchecked,
        jwks_uri: Some(format!("{issuer_uri}/jwks")),
        algorithms: vec![Algorithm::ES256],
        jwks_ttl: std::time::Duration::from_secs(300),
        clock_skew: std::time::Duration::from_secs(60),
        min_refetch_interval: std::time::Duration::from_secs(60),
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
    fn now_secs_is_plausibly_recent() {
        assert!(now_secs() > 1_577_836_800);
    }
}
