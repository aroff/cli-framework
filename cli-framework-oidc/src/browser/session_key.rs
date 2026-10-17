use crate::OidcConfigError;
use hkdf::Hkdf;
use sha2::Sha256;
use std::sync::Arc;
use zeroize::Zeroize;

struct KeyBytes([u8; 32]);

impl Drop for KeyBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// 32-byte AES-256-GCM session key, safe to clone across Tower worker tasks.
///
/// Internally uses `Arc` so cloning is cheap and the bytes are zeroized when
/// the last clone is dropped. No `Debug`, no `Serialize`/`Deserialize`.
#[derive(Clone)]
pub struct SessionKey(Arc<KeyBytes>);

impl SessionKey {
    /// The shortest secret [`derive`](Self::derive) accepts, in bytes.
    pub const MIN_SECRET_LEN: usize = 32;

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Arc::new(KeyBytes(bytes)))
    }

    /// A key derived with HKDF-SHA256 from a generated secret, for a
    /// deployment whose secret store holds a random string rather than 32 raw
    /// bytes. `context` says what the key is for (e.g. `b"acme host session
    /// v1"`): the same secret with another context gives an unrelated key.
    ///
    /// The secret is used as given, so trim a file's trailing newline first.
    /// Refuses a secret shorter than [`MIN_SECRET_LEN`](Self::MIN_SECRET_LEN)
    /// bytes; the error names its length, never its bytes. The same secret
    /// and context always give the same key, so changing either signs every
    /// session out.
    pub fn derive(secret: &[u8], context: &[u8]) -> Result<Self, OidcConfigError> {
        if secret.len() < Self::MIN_SECRET_LEN {
            return Err(OidcConfigError::ShortSessionSecret(secret.len()));
        }
        let mut key = [0u8; 32];
        Hkdf::<Sha256>::new(None, secret)
            .expand(context, &mut key)
            .expect("HKDF expand: 32 bytes always fits");
        let derived = Self::from_bytes(key);
        key.zeroize();
        Ok(derived)
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0 .0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    #[test]
    fn derive_gives_hkdf_sha256_of_the_secret_for_the_context() {
        // HKDF-SHA256, no salt, info = the context, 32 bytes (RFC 5869),
        // computed independently. Pinned: a change here would sign every
        // session of every deployment out.
        let key = SessionKey::derive(SECRET, b"test host session v1").unwrap();
        let hex: String = key.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "d0e55e2ac06e1ea80c1c087f7b54dae770444e3bf4a682e6e1ee68e8b5d44508"
        );
    }

    #[test]
    fn derive_depends_on_both_the_secret_and_the_context() {
        let key = |secret: &[u8], context: &[u8]| {
            *SessionKey::derive(secret, context).unwrap().as_bytes()
        };
        assert_eq!(key(SECRET, b"a"), key(SECRET, b"a"));
        assert_ne!(key(SECRET, b"a"), key(SECRET, b"b"));
        assert_ne!(
            key(SECRET, b"a"),
            key(b"0123456789abcdef0123456789abcdeF", b"a")
        );
        assert_ne!(&key(SECRET, b"a")[..], SECRET);
    }

    #[test]
    fn derive_refuses_a_short_secret_without_naming_it() {
        let short = b"short-secret-canary-0123456789a";
        assert_eq!(short.len(), SessionKey::MIN_SECRET_LEN - 1);
        let error = SessionKey::derive(short, b"a").err().unwrap().to_string();
        assert!(error.contains("31 bytes"), "{error}");
        assert!(!error.contains("canary"), "{error}");
        assert!(SessionKey::derive(b"", b"a").is_err());
        assert!(SessionKey::derive(SECRET, b"a").is_ok());
    }
}
