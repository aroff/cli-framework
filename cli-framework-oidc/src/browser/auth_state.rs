/// HMAC-SHA256 signed auth-state cookie for PKCE CSRF protection.
///
/// Format: base64url(payload_json).base64url(hmac_sha256)
/// Payload: { "s": "<state>", "v": "<pkce_verifier>", "r": "<return_to>", "iat": <seconds> }
///
/// The HMAC key is derived from session_key via HKDF-SHA256 with info="auth_state_hmac".
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub struct AuthState {
    /// Opaque random state value sent to Keycloak.
    pub state: String,
    /// PKCE code verifier — stays in cookie, never sent to Keycloak.
    pub verifier: String,
    /// Path the user was trying to reach before the login redirect.
    pub return_to: String,
}

/// Derive the HMAC key from the session key.
pub fn derive_hmac_key(session_key: &[u8; 32]) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(None, session_key);
    let mut key = [0u8; 32];
    hkdf.expand(b"auth_state_hmac", &mut key)
        .expect("HKDF expand: 32 bytes always fits");
    key
}

/// Encode an AuthState into a signed cookie value.
pub fn encode_auth_state(state: &AuthState, hmac_key: &[u8; 32]) -> String {
    encode_at(state, hmac_key, now_secs())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn encode_at(state: &AuthState, hmac_key: &[u8; 32], issued_at: u64) -> String {
    let payload = serde_json::json!({
        "s": state.state,
        "v": state.verifier,
        "r": state.return_to,
        "iat": issued_at,
    });
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());

    let mut mac = HmacSha256::new_from_slice(hmac_key).expect("HMAC accepts any key size");
    mac.update(payload_b64.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = URL_SAFE_NO_PAD.encode(sig.as_slice());

    format!("{payload_b64}.{sig_b64}")
}

/// Decode and verify a signed cookie value, returning the AuthState.
/// Returns `None` on any verification failure (tampered, malformed, missing).
pub fn decode_auth_state(cookie_value: &str, hmac_key: &[u8; 32]) -> Option<AuthState> {
    decode_at(cookie_value, hmac_key, now_secs())
}

fn decode_at(cookie_value: &str, hmac_key: &[u8; 32], now: u64) -> Option<AuthState> {
    if cookie_value.len() > 3800 {
        return None;
    }
    let dot = cookie_value.rfind('.')?;
    let payload_b64 = &cookie_value[..dot];
    let sig_b64 = &cookie_value[dot + 1..];

    let expected_sig = URL_SAFE_NO_PAD.decode(sig_b64).ok()?;
    let mut mac = HmacSha256::new_from_slice(hmac_key).expect("HMAC accepts any key size");
    mac.update(payload_b64.as_bytes());
    mac.verify_slice(&expected_sig).ok()?;

    let payload_bytes = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;

    let issued_at = v["iat"].as_u64()?;
    if issued_at > now || now - issued_at >= 600 {
        return None;
    }
    let result = AuthState {
        state: v["s"].as_str()?.to_string(),
        verifier: v["v"].as_str()?.to_string(),
        return_to: v["r"].as_str()?.to_string(),
    };
    if result.state.is_empty()
        || result.state.len() > 128
        || result.verifier.is_empty()
        || result.verifier.len() > 128
        || result.return_to.len() > 1024
        || super::request_type::validate_return_to(&result.return_to).is_err()
    {
        return None;
    }
    Some(result)
}

/// Generate a cryptographically random opaque state string (URL-safe base64, 32 random bytes).
pub fn random_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_state_has_a_strict_lifetime_and_safe_return_path() {
        let key = [7; 32];
        let mut state = AuthState {
            state: "state".into(),
            verifier: "verifier".into(),
            return_to: "/review?tab=files".into(),
        };
        let signed = encode_at(&state, &key, 1000);
        assert!(decode_at(&signed, &key, 999).is_none());
        assert!(decode_at(&signed, &key, 1599).is_some());
        assert!(decode_at(&signed, &key, 1600).is_none());
        state.return_to = "//other.example".into();
        assert!(decode_at(&encode_at(&state, &key, 1000), &key, 1000).is_none());
        assert!(decode_at(&"x".repeat(3801), &key, 1000).is_none());
    }
}
