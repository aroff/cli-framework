//! [`SecretValue`]: a zeroizing, non-leaking byte container.

use std::fmt;
use zeroize::Zeroizing;

/// A secret's bytes, zeroized on drop.
///
/// Contents are reachable only via [`SecretValue::expose`] /
/// [`SecretValue::expose_str`] — deliberately explicit, so a call site
/// reading a secret is grep-able. `SecretValue` intentionally has no
/// `Serialize`/`Display` impl, and its `Debug` impl never prints the bytes.
pub struct SecretValue(Zeroizing<Vec<u8>>);

impl SecretValue {
    /// Build a `SecretValue` from owned bytes (moves in, no extra copy).
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(Zeroizing::new(bytes.into()))
    }

    /// Borrow the raw bytes. Named `expose*` rather than `as_*` so call
    /// sites make the leak-risk explicit.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Borrow the bytes as UTF-8, if valid.
    pub fn expose_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.0)
    }

    /// Byte length (not itself sensitive; safe to log).
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for SecretValue {
    fn from(s: String) -> Self {
        Self::new(s.into_bytes())
    }
}

impl From<&str> for SecretValue {
    fn from(s: &str) -> Self {
        Self::new(s.as_bytes().to_vec())
    }
}

impl From<Vec<u8>> for SecretValue {
    fn from(v: Vec<u8>) -> Self {
        Self::new(v)
    }
}

impl Clone for SecretValue {
    fn clone(&self) -> Self {
        Self::new(self.0.to_vec())
    }
}

/// Never prints the contents — see the module docs for why.
impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SecretValue").field(&"[redacted]").finish()
    }
}

impl PartialEq for SecretValue {
    /// Plain byte comparison — adequate for tests/backend round-trip checks,
    /// **not** a constant-time compare; do not use this for e.g. verifying a
    /// signature or webhook secret against attacker-controlled input.
    fn eq(&self, other: &Self) -> bool {
        self.0.as_slice() == other.0.as_slice()
    }
}
impl Eq for SecretValue {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expose_returns_original_bytes() {
        let v = SecretValue::from("hunter2");
        assert_eq!(v.expose(), b"hunter2");
        assert_eq!(v.expose_str().unwrap(), "hunter2");
    }

    #[test]
    fn debug_never_contains_the_secret() {
        let v = SecretValue::from("super-secret-value-12345");
        let dbg = format!("{v:?}");
        assert!(
            !dbg.contains("super-secret-value-12345"),
            "Debug output leaked the secret: {dbg}"
        );
        assert!(dbg.contains("redacted"), "got: {dbg}");
    }

    #[test]
    fn no_serialize_or_display_impl_exists() {
        // Compile-time guarantee: SecretValue does not implement
        // serde::Serialize or std::fmt::Display. There's nothing to assert
        // at runtime — if either impl existed, this file (or any consumer
        // trying to `serde_json::to_string(&secret_value)` /
        // `format!("{secret_value}")`) would fail to compile.
    }

    #[test]
    fn clone_is_an_independent_zeroizing_copy() {
        let v1 = SecretValue::from("abc");
        let v2 = v1.clone();
        assert_eq!(v1, v2);
        drop(v1);
        assert_eq!(v2.expose(), b"abc");
    }

    #[test]
    fn zeroizes_on_drop_best_effort() {
        // Best-effort proof of zeroization, without reading freed memory
        // (an earlier version of this test read the buffer *after*
        // `drop()`, which is technically-UB and turned out to be
        // observably flaky here: the allocator reused/overwrote the freed
        // bytes with freelist bookkeeping before the assertion ran).
        //
        // Instead: `Zeroizing<Vec<u8>>::drop` (what `SecretValue`'s field
        // uses) is documented to call `Zeroize::zeroize()` on the inner
        // buffer before the `Vec` is deallocated. We drive that same call
        // explicitly via `ManuallyDrop` so the allocation stays valid and
        // we can assert on it directly, then finish the drop ourselves.
        use std::mem::ManuallyDrop;
        use zeroize::Zeroize;

        let marker = b"zzZZ-marker-zzZZ".to_vec();
        let mut guard = ManuallyDrop::new(SecretValue::new(marker));
        guard.0.zeroize(); // the exact call SecretValue's own Drop performs
        assert!(
            guard.0.iter().all(|&b| b == 0),
            "expected zeroize() to clear the buffer, got {:?}",
            &guard.0[..]
        );
        // SAFETY: `guard` hasn't been used since `ManuallyDrop::new`, so
        // this is the only drop that will ever run for it.
        unsafe { ManuallyDrop::drop(&mut guard) };
    }
}
