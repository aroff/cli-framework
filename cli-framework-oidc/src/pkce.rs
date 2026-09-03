//! PKCE (RFC 7636) verifier/challenge generation and the OAuth `state` nonce.
//!
//! The `client` and `browser` features both need the same three primitives, and
//! both need them backed by the OS CSPRNG. They live at the crate root rather
//! than inside either feature's module so there is exactly one implementation to
//! audit — `browser::pkce` is a re-export of this module, not a second copy.
//!
//! Every value here is security-relevant: the verifier is the only thing binding
//! a redeemed authorization or device code to the process that requested it, and
//! `state` is the CSRF defence on the browser redirect. Both must come from
//! `OsRng`. A time-plus-counter PRNG is not adequate — an attacker who can
//! observe or approximate the request time recovers most of the state space.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Fill `n` bytes from the OS CSPRNG.
fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    buf
}

/// Generate a cryptographically random PKCE code verifier: 32 random bytes
/// rendered as 43 URL-safe base64 characters, which is RFC 7636 §4.1's
/// recommended length and well inside its 43–128 character range.
pub fn generate_verifier() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes(32))
}

/// Derive the S256 code challenge from a verifier: `base64url(SHA-256(verifier))`
/// (RFC 7636 §4.2). The hash is taken over the ASCII verifier *string*, not over
/// the bytes it was encoded from.
pub fn derive_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash.as_slice())
}

/// Generate a random OAuth `state` value: 16 random bytes as 32 lowercase hex
/// characters (128 bits).
pub fn generate_state() -> String {
    random_bytes(16)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
