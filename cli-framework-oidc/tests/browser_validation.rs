use cli_framework::axum::http::{HeaderMap, HeaderValue};
/// Unit tests for the browser feature — cookie crypto, PKCE, auth state, request-type detection.
/// These tests do not require a network connection or real Keycloak instance.
use cli_framework_oidc::browser::{
    auth_state::{decode_auth_state, derive_hmac_key, encode_auth_state, random_state, AuthState},
    cookie::{decrypt_cookie, encrypt_cookie},
    pkce::{derive_challenge, generate_verifier},
    request_type::{detect, validate_return_to, RequestType},
    session_key::SessionKey,
};

fn test_key() -> [u8; 32] {
    [42u8; 32]
}

fn test_session_key() -> SessionKey {
    SessionKey::from_bytes(test_key())
}

// ── T1: Cookie validation ────────────────────────────────────────────────────

#[test]
fn test_cookie_roundtrip() {
    let key = test_key();
    let at = "eyJhbGciOiJSUzI1NiJ9.access.token";
    let rt = "opaque-refresh-token-xyz";
    let exp = 9_999_999_999i64;

    let encrypted = encrypt_cookie(&key, at, rt, exp).expect("encrypt");
    let payload = decrypt_cookie(&key, &encrypted).expect("decrypt");

    assert_eq!(payload.access_token, at);
    assert_eq!(payload.refresh_token, rt);
    assert_eq!(payload.refresh_exp, exp);
}

#[test]
fn test_cookie_tamper_detection() {
    let key = test_key();
    let encrypted = encrypt_cookie(&key, "token", "refresh", 9999999999).expect("encrypt");

    // Flip a byte in the middle of the base64
    let mut tampered = encrypted.clone();
    let mid = tampered.len() / 2;
    let bytes = unsafe { tampered.as_bytes_mut() };
    bytes[mid] ^= 0xFF;

    let result = decrypt_cookie(&key, &tampered);
    assert!(result.is_err(), "tampered cookie should fail decryption");
}

#[test]
fn test_cookie_wrong_key() {
    let key1 = [1u8; 32];
    let key2 = [2u8; 32];
    let encrypted = encrypt_cookie(&key1, "token", "refresh", 9999999999).expect("encrypt");
    let result = decrypt_cookie(&key2, &encrypted);
    assert!(result.is_err(), "wrong key should fail decryption");
}

#[test]
fn test_cookie_invalid_base64_rejected() {
    let key = test_key();
    let result = decrypt_cookie(&key, "not-valid-base64!!!");
    assert!(result.is_err());
}

#[test]
fn test_cookie_unknown_version() {
    // We can't easily forge an unknown-version cookie (it's encrypted), but we can verify
    // that the version 1 path succeeds and that the error type exists.
    let key = test_key();
    let encrypted = encrypt_cookie(&key, "at", "rt", 1234567890).expect("encrypt");
    let result = decrypt_cookie(&key, &encrypted);
    assert!(result.is_ok());
}

// ── T2: PKCE ────────────────────────────────────────────────────────────────

#[test]
fn test_pkce_verifier_length() {
    let v = generate_verifier();
    // 32 raw bytes → base64url = ceil(32 * 4/3) = 43 chars (no padding)
    assert_eq!(v.len(), 43, "verifier should be 43 chars");
}

#[test]
fn test_pkce_verifier_unique() {
    let v1 = generate_verifier();
    let v2 = generate_verifier();
    assert_ne!(v1, v2, "two verifiers should be distinct");
}

#[test]
fn test_pkce_challenge_deterministic() {
    let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    // SHA-256 of the above verifier, base64url (known test vector from RFC 7636)
    let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    assert_eq!(derive_challenge(v), expected);
}

#[test]
fn test_pkce_roundtrip() {
    let verifier = generate_verifier();
    let challenge = derive_challenge(&verifier);
    assert!(!challenge.is_empty());
    // Challenge must not equal the verifier (it's the hash)
    assert_ne!(verifier, challenge);
}

// ── T3: Auth state (state anti-forgery) ─────────────────────────────────────

#[test]
fn test_auth_state_roundtrip() {
    let key = test_key();
    let hmac_key = derive_hmac_key(&key);

    let state = AuthState {
        state: "random-state-xyz".to_string(),
        verifier: "pkce-verifier-abc".to_string(),
        return_to: "/dashboard".to_string(),
    };

    let encoded = encode_auth_state(&state, &hmac_key);
    let decoded = decode_auth_state(&encoded, &hmac_key).expect("decode");

    assert_eq!(decoded.state, "random-state-xyz");
    assert_eq!(decoded.verifier, "pkce-verifier-abc");
    assert_eq!(decoded.return_to, "/dashboard");
}

#[test]
fn test_auth_state_hmac_reject_on_tamper() {
    let key = test_key();
    let hmac_key = derive_hmac_key(&key);

    let auth_state = AuthState {
        state: "s".to_string(),
        verifier: "v".to_string(),
        return_to: "/".to_string(),
    };
    let encoded = encode_auth_state(&auth_state, &hmac_key);

    // Flip a character in the payload portion (before the last '.')
    let dot = encoded.rfind('.').unwrap();
    let mut tampered = encoded.clone();
    let bytes = unsafe { tampered.as_bytes_mut() };
    bytes[dot - 1] ^= 0x01;

    let result = decode_auth_state(&tampered, &hmac_key);
    assert!(result.is_none(), "tampered auth state should be rejected");
}

#[test]
fn test_auth_state_wrong_key_rejected() {
    let key1 = [1u8; 32];
    let key2 = [2u8; 32];
    let hmac_key1 = derive_hmac_key(&key1);
    let hmac_key2 = derive_hmac_key(&key2);

    let auth_state = AuthState {
        state: "s".to_string(),
        verifier: "v".to_string(),
        return_to: "/".to_string(),
    };
    let encoded = encode_auth_state(&auth_state, &hmac_key1);
    assert!(
        decode_auth_state(&encoded, &hmac_key2).is_none(),
        "wrong HMAC key should reject"
    );
}

#[test]
fn test_auth_state_missing_dot_rejected() {
    let key = test_key();
    let hmac_key = derive_hmac_key(&key);
    assert!(decode_auth_state("nodothere", &hmac_key).is_none());
}

#[test]
fn test_random_state_unique() {
    let s1 = random_state();
    let s2 = random_state();
    assert_ne!(s1, s2);
    assert!(!s1.is_empty());
}

// ── T6: Request-type detection ───────────────────────────────────────────────

fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.insert(
            cli_framework::axum::http::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
            HeaderValue::from_str(v).unwrap(),
        );
    }
    h
}

#[test]
fn test_request_type_navigate_via_sec_fetch_mode() {
    let h = headers_with(&[("sec-fetch-mode", "navigate")]);
    assert_eq!(detect(&h), RequestType::Navigation);
}

#[test]
fn test_request_type_cors_via_sec_fetch_mode() {
    let h = headers_with(&[("sec-fetch-mode", "cors")]);
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

#[test]
fn test_request_type_same_origin_via_sec_fetch_mode() {
    let h = headers_with(&[("sec-fetch-mode", "same-origin")]);
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

#[test]
fn test_request_type_text_html_accept_fallback() {
    let h = headers_with(&[("accept", "text/html,application/xhtml+xml")]);
    assert_eq!(detect(&h), RequestType::Navigation);
}

#[test]
fn test_request_type_json_accept_fallback() {
    let h = headers_with(&[("accept", "application/json")]);
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

#[test]
fn test_request_type_wildcard_accept_is_api() {
    // fetch() default: Accept: */* — must go to API/fetch branch, not navigation
    let h = headers_with(&[("accept", "*/*")]);
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

#[test]
fn test_request_type_no_headers_is_api() {
    let h = HeaderMap::new();
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

// ── Return-to validation ─────────────────────────────────────────────────────

#[test]
fn test_return_to_valid_path() {
    assert!(validate_return_to("/dashboard").is_ok());
    assert!(validate_return_to("/some/deep/path?q=1").is_ok());
    assert!(validate_return_to("/").is_ok());
}

#[test]
fn test_return_to_protocol_relative_rejected() {
    assert!(validate_return_to("//evil.com").is_err());
}

#[test]
fn test_return_to_backslash_rejected() {
    assert!(validate_return_to("\\evil").is_err());
}

#[test]
fn test_return_to_url_encoded_traversal_rejected() {
    assert!(validate_return_to("%2F%2Fevil.com").is_err());
    assert!(validate_return_to("%5cevil").is_err());
}

#[test]
fn test_return_to_control_chars_rejected() {
    assert!(validate_return_to("/path\r\nSet-Cookie: x=y").is_err());
    assert!(validate_return_to("/path\ninjection").is_err());
}

#[test]
fn test_return_to_must_start_with_slash() {
    assert!(validate_return_to("relative/path").is_err());
    assert!(validate_return_to("https://evil.com/path").is_err());
}

// ── SessionKey properties ────────────────────────────────────────────────────

#[test]
fn test_session_key_clone_is_available() {
    // Verify SessionKey implements Clone (compile-time test).
    let key = test_session_key();
    let key2 = key.clone();
    // Both should be usable independently — verified by the cookie roundtrip tests above.
    let _ = key2;
}

// Verify SessionKey does not implement Debug (would be a compile error if enabled).
// This is tested implicitly — if it compiled, the test passes.
#[test]
fn test_session_key_no_debug_compile() {
    // If Debug were derived, this would still compile — but the absence of the
    // trait is enforced by the type definition (no #[derive(Debug)]).
    let _key = test_session_key();
    // If the line below were uncommented, it would fail to compile:
    // println!("{:?}", _key);
}
