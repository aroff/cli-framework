//! [`SecretError`]: typed errors for [`super::SecretStore`] implementations.

/// Errors a [`super::SecretStore`] backend can return.
///
/// Deliberately typed (rather than an opaque `anyhow::Error`) so callers can
/// branch: retry on [`SecretError::Unavailable`], fail closed on
/// [`SecretError::PermissionDenied`], fall back on [`SecretError::NotFound`].
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum SecretError {
    /// No value stored under this key.
    #[error("secret not found")]
    NotFound,

    /// Transient/retryable backend failure (network blip, backend down).
    #[error("secret backend unavailable: {0}")]
    Unavailable(String),

    /// The backend understood the request but refused it (auth/ACL).
    #[error("permission denied")]
    PermissionDenied,

    /// The operation isn't implemented by this backend (e.g. `rotate` on a
    /// static store).
    #[error("operation not supported: {0}")]
    NotSupported(&'static str),

    /// Any other backend-specific failure, with the underlying error
    /// preserved via `#[source]`.
    #[error("secret backend error: {source}")]
    Backend {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl SecretError {
    /// Wrap an arbitrary backend error as [`SecretError::Backend`].
    pub fn backend(err: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        SecretError::Backend { source: err.into() }
    }
}
