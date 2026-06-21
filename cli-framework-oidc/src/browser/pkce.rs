/// PKCE (RFC 7636) code verifier and challenge generation.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Generate a cryptographically random PKCE code verifier (43 URL-safe base64 chars = 32 bytes).
pub fn generate_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Derive the S256 code challenge from a verifier: base64url(SHA-256(verifier)).
pub fn derive_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash.as_slice())
}
