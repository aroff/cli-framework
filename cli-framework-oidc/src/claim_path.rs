//! Claim paths: where in a verified token's claims to read roles or groups.
//!
//! A path is a list of object keys separated by `.`, descending through
//! nested JSON objects: `realm_access.roles` reads `claims["realm_access"]["roles"]`.
//! A key that itself contains a dot is written with the dot escaped as `\.`
//! (`https://example\.com/roles` is the single key `https://example.com/roles`),
//! and a literal backslash as `\\`. Any other backslash sequence, a trailing
//! backslash, an empty key or an empty path is a configuration error.
//!
//! The value found at the path is read as a list of strings: an array keeps
//! its string elements (others are ignored), a single string becomes a
//! one-element list, and anything else — or a path that does not resolve —
//! yields an empty list rather than an error.

use crate::OidcConfigError;
use serde_json::Value as JsonValue;

/// Default roles claim path: the realm roles a Keycloak access token carries.
pub const DEFAULT_ROLES_CLAIM_PATH: &str = "realm_access.roles";

/// A parsed claim path: the object keys to descend through, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClaimPath {
    keys: Vec<String>,
}

impl ClaimPath {
    /// Parse a dot-separated path, honouring `\.` and `\\` escapes.
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub(crate) fn parse(raw: &str) -> Result<Self, OidcConfigError> {
        let invalid = |why: &str| OidcConfigError::InvalidClaimPath(format!("{raw:?}: {why}"));
        if raw.is_empty() {
            return Err(invalid("path is empty"));
        }
        let mut keys = Vec::new();
        let mut current = String::new();
        let mut chars = raw.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('.') => current.push('.'),
                    Some('\\') => current.push('\\'),
                    Some(other) => {
                        return Err(invalid(&format!("unsupported escape \\{other}")));
                    }
                    None => return Err(invalid("trailing backslash")),
                },
                '.' => {
                    if current.is_empty() {
                        return Err(invalid("empty key"));
                    }
                    keys.push(std::mem::take(&mut current));
                }
                other => current.push(other),
            }
        }
        if current.is_empty() {
            return Err(invalid("empty key"));
        }
        keys.push(current);
        Ok(Self { keys })
    }

    /// The default roles path, [`DEFAULT_ROLES_CLAIM_PATH`] (used by the
    /// browser layers, which do not take claim-path configuration).
    #[cfg_attr(not(feature = "browser"), allow(dead_code))]
    pub(crate) fn default_roles() -> Self {
        Self {
            keys: vec!["realm_access".to_string(), "roles".to_string()],
        }
    }

    /// The strings at this path in `claims` (see the module docs for the rules).
    pub(crate) fn strings(&self, claims: &JsonValue) -> Vec<String> {
        let mut node = claims;
        for key in &self.keys {
            match node.as_object().and_then(|o| o.get(key)) {
                Some(next) => node = next,
                None => return Vec::new(),
            }
        }
        match node {
            JsonValue::Array(items) => items
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            JsonValue::String(s) => vec![s.clone()],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(raw: &str) -> Vec<String> {
        ClaimPath::parse(raw).unwrap().keys
    }

    #[test]
    fn parses_plain_and_escaped_keys() {
        assert_eq!(keys("realm_access.roles"), ["realm_access", "roles"]);
        assert_eq!(keys("groups"), ["groups"]);
        assert_eq!(
            keys(r"https://example\.com/claims.roles"),
            ["https://example.com/claims", "roles"]
        );
        assert_eq!(keys(r"a\\b.c"), [r"a\b", "c"]);
        assert_eq!(
            ClaimPath::default_roles(),
            ClaimPath::parse(DEFAULT_ROLES_CLAIM_PATH).unwrap()
        );
    }

    #[test]
    fn rejects_malformed_paths() {
        for bad in ["", ".", "a.", ".a", "a..b", r"a\", r"a\x"] {
            assert!(
                matches!(
                    ClaimPath::parse(bad),
                    Err(OidcConfigError::InvalidClaimPath(_))
                ),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn reads_arrays_single_strings_and_ignores_the_rest() {
        let claims = json!({
            "org": {"roles": ["x:viewer", 7, null, "x:editor"], "one": "solo", "num": 3},
            "flat.key": ["dotted"],
            "list": [{"roles": ["nope"]}],
        });
        let at = |p: &str| ClaimPath::parse(p).unwrap().strings(&claims);
        assert_eq!(at("org.roles"), ["x:viewer", "x:editor"]);
        assert_eq!(at("org.one"), ["solo"]);
        assert!(at("org.num").is_empty());
        assert!(at("org.missing").is_empty());
        assert!(at("missing.roles").is_empty());
        assert!(at("org.roles.deeper").is_empty());
        assert!(at("list.roles").is_empty(), "arrays are not descended into");
        assert_eq!(at(r"flat\.key"), ["dotted"]);
    }
}
