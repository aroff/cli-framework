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
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Arc::new(KeyBytes(bytes)))
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0 .0
    }
}
