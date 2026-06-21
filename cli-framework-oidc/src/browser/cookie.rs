/// AES-256-GCM cookie encryption for browser session tokens.
///
/// Encryption scheme (per spec §AES-GCM Cryptographic Construction):
///   Each field (access token, refresh token) is independently encrypted:
///     blob = nonce (12 bytes) || GCM_ciphertext_with_tag
///   The outer cookie is the JSON envelope encrypted the same way and base64url-encoded.
///
/// Cookie JSON (plaintext before outer encryption):
///   { "v": 1, "at": "<b64url(nonce||ct_at)>", "rt": "<b64url(nonce||ct_rt)>", "exp": <i64> }
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CookieError {
    #[error("cookie is tampered or MAC-invalid")]
    Tampered,
    #[error("cookie is malformed")]
    Invalid,
    #[error("unknown cookie version: {0}")]
    UnknownVersion(u8),
    #[error("crypto error")]
    Crypto,
}

/// Decrypted cookie payload.
pub struct CookiePayload {
    pub access_token: String,
    pub refresh_token: String,
    /// Refresh token expiry as Unix seconds (governs cookie Max-Age).
    pub refresh_exp: i64,
}

#[derive(Serialize, Deserialize)]
struct CookieJson {
    v: u8,
    at: String,
    rt: String,
    exp: i64,
}

fn random_nonce() -> [u8; 12] {
    let mut n = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut n);
    n
}

fn aes_encrypt(cipher: &Aes256Gcm, plaintext: &[u8]) -> Result<Vec<u8>, CookieError> {
    let nonce_bytes = random_nonce();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CookieError::Crypto)?;
    let mut blob = Vec::with_capacity(12 + ct.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ct);
    Ok(blob)
}

fn aes_decrypt(cipher: &Aes256Gcm, blob: &[u8]) -> Result<Vec<u8>, CookieError> {
    if blob.len() < 12 {
        return Err(CookieError::Invalid);
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher.decrypt(nonce, ct).map_err(|_| CookieError::Tampered)
}

/// Encrypt access_token and refresh_token into a sealed cookie value.
pub fn encrypt_cookie(
    key: &[u8; 32],
    access_token: &str,
    refresh_token: &str,
    refresh_exp: i64,
) -> Result<String, CookieError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CookieError::Crypto)?;

    let at_blob = aes_encrypt(&cipher, access_token.as_bytes())?;
    let rt_blob = aes_encrypt(&cipher, refresh_token.as_bytes())?;

    let at_b64 = URL_SAFE_NO_PAD.encode(&at_blob);
    let rt_b64 = URL_SAFE_NO_PAD.encode(&rt_blob);

    let inner = serde_json::to_string(&CookieJson {
        v: 1,
        at: at_b64,
        rt: rt_b64,
        exp: refresh_exp,
    })
    .map_err(|_| CookieError::Crypto)?;

    let outer_blob = aes_encrypt(&cipher, inner.as_bytes())?;
    Ok(URL_SAFE_NO_PAD.encode(&outer_blob))
}

/// Decrypt and verify a cookie value, returning the contained tokens.
pub fn decrypt_cookie(key: &[u8; 32], cookie_value: &str) -> Result<CookiePayload, CookieError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CookieError::Crypto)?;

    let outer_blob = URL_SAFE_NO_PAD
        .decode(cookie_value)
        .map_err(|_| CookieError::Invalid)?;
    let inner_bytes = aes_decrypt(&cipher, &outer_blob)?;

    let json: CookieJson =
        serde_json::from_slice(&inner_bytes).map_err(|_| CookieError::Invalid)?;
    if json.v != 1 {
        return Err(CookieError::UnknownVersion(json.v));
    }

    let at_blob = URL_SAFE_NO_PAD
        .decode(&json.at)
        .map_err(|_| CookieError::Invalid)?;
    let at_bytes = aes_decrypt(&cipher, &at_blob)?;
    let access_token = String::from_utf8(at_bytes).map_err(|_| CookieError::Invalid)?;

    let rt_blob = URL_SAFE_NO_PAD
        .decode(&json.rt)
        .map_err(|_| CookieError::Invalid)?;
    let rt_bytes = aes_decrypt(&cipher, &rt_blob)?;
    let refresh_token = String::from_utf8(rt_bytes).map_err(|_| CookieError::Invalid)?;

    Ok(CookiePayload {
        access_token,
        refresh_token,
        refresh_exp: json.exp,
    })
}

/// Estimate the encrypted cookie size for a given access token length.
/// Used at startup for the CookieTooLarge check.
pub fn estimate_cookie_size(key: &[u8; 32], access_token_len: usize) -> usize {
    // Synthetic tokens of the given size.
    let at = "A".repeat(access_token_len);
    let rt = "R".repeat(64); // typical opaque refresh token
    let exp = 9_999_999_999i64;
    match encrypt_cookie(key, &at, &rt, exp) {
        Ok(s) => s.len(),
        Err(_) => usize::MAX,
    }
}
