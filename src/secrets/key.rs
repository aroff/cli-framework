//! [`SecretKey`]: a namespaced, validated secret path.

use std::fmt;

/// A namespaced, validated secret path: segments joined by `/`.
///
/// Segment charset: ASCII alphanumerics, `_`, `-`, `.`. Segments may not be
/// empty, and a segment that is exactly `.` or `..` is rejected so keys can't
/// take on path-traversal shapes once a filesystem-backed [`super::SecretStore`]
/// maps them onto a directory tree.
///
/// ```
/// use cli_framework::secrets::SecretKey;
///
/// let k = SecretKey::new(["connector_app", "salesforce", "oauth2", "client_secret"]).unwrap();
/// assert_eq!(k.as_str(), "connector_app/salesforce/oauth2/client_secret");
///
/// let k2 = SecretKey::parse("connection/42/refresh_token").unwrap();
/// assert_eq!(k2.segments().count(), 3);
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SecretKey(String);

/// Rejected key construction.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SecretKeyError {
    #[error("secret key must have at least one segment")]
    Empty,
    #[error("secret key segment must not be empty")]
    EmptySegment,
    #[error(
        "invalid secret key segment {0:?}: only [A-Za-z0-9._-] allowed, and '.'/'..' are reserved"
    )]
    InvalidSegment(String),
}

impl SecretKey {
    /// Build a key from an ordered list of segments, validating each one.
    pub fn new<I, S>(segments: I) -> Result<Self, SecretKeyError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut joined = String::new();
        let mut any = false;
        for seg in segments {
            let s = seg.as_ref();
            validate_segment(s)?;
            if any {
                joined.push('/');
            }
            joined.push_str(s);
            any = true;
        }
        if !any {
            return Err(SecretKeyError::Empty);
        }
        Ok(Self(joined))
    }

    /// Parse an already-`/`-joined path, validating each segment.
    pub fn parse(path: &str) -> Result<Self, SecretKeyError> {
        Self::new(path.split('/'))
    }

    /// The full `/`-joined path.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Iterate the individual segments.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

fn validate_segment(seg: &str) -> Result<(), SecretKeyError> {
    if seg.is_empty() {
        return Err(SecretKeyError::EmptySegment);
    }
    if seg == "." || seg == ".." {
        return Err(SecretKeyError::InvalidSegment(seg.to_string()));
    }
    if !seg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(SecretKeyError::InvalidSegment(seg.to_string()));
    }
    Ok(())
}

impl fmt::Display for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SecretKey").field(&self.0).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_segments_with_slash() {
        let k = SecretKey::new(["a", "b", "c"]).unwrap();
        assert_eq!(k.as_str(), "a/b/c");
        assert_eq!(k.segments().collect::<Vec<_>>(), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_round_trips_with_new() {
        let k1 = SecretKey::parse("connection/42/refresh_token").unwrap();
        let k2 = SecretKey::new(["connection", "42", "refresh_token"]).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(
            SecretKey::new(Vec::<&str>::new()),
            Err(SecretKeyError::Empty)
        );
    }

    #[test]
    fn rejects_empty_segment() {
        assert_eq!(
            SecretKey::new(["a", "", "b"]),
            Err(SecretKeyError::EmptySegment)
        );
    }

    #[test]
    fn rejects_dot_segments() {
        assert!(SecretKey::new(["a", "..", "b"]).is_err());
        assert!(SecretKey::new(["."]).is_err());
    }

    #[test]
    fn rejects_unsafe_charset() {
        assert!(SecretKey::new(["a/b"]).is_err()); // embedded slash in one segment
        assert!(SecretKey::new(["a b"]).is_err()); // space
        assert!(SecretKey::new(["a$b"]).is_err()); // shell-meaningful char
    }

    #[test]
    fn allows_dots_dashes_underscores() {
        assert!(SecretKey::new(["oidc-token.json"]).is_ok());
        assert!(SecretKey::new(["client_secret-v2"]).is_ok());
    }

    #[test]
    fn debug_and_display_show_the_path_not_a_placeholder() {
        // SecretKey is a name, not a secret — it's fine (expected) for it to
        // print in full via Debug/Display.
        let k = SecretKey::new(["oauth", "state_signing_key"]).unwrap();
        assert_eq!(format!("{k}"), "oauth/state_signing_key");
        assert!(format!("{k:?}").contains("oauth/state_signing_key"));
    }
}
