/// HMAC-SHA256 signed auth-state cookie for PKCE CSRF protection.
///
/// Format: base64url(payload_json).base64url(hmac_sha256)
/// Payload: { "s": "<state>", "v": "<pkce_verifier>", "r": "<return_to>" }
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
    let payload = serde_json::json!({
        "s": state.state,
        "v": state.verifier,
        "r": state.return_to,
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
    let dot = cookie_value.rfind('.')?;
    let payload_b64 = &cookie_value[..dot];
    let sig_b64 = &cookie_value[dot + 1..];

    let expected_sig = URL_SAFE_NO_PAD.decode(sig_b64).ok()?;
    let mut mac = HmacSha256::new_from_slice(hmac_key).expect("HMAC accepts any key size");
    mac.update(payload_b64.as_bytes());
    mac.verify_slice(&expected_sig).ok()?;

    let payload_bytes = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;

    Some(AuthState {
        state: v["s"].as_str()?.to_string(),
        verifier: v["v"].as_str()?.to_string(),
        return_to: v["r"].as_str()?.to_string(),
    })
}

/// Generate a cryptographically random opaque state string (URL-safe base64, 32 random bytes).
pub fn random_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
