//! Shared validation for OIDC endpoints that may receive credentials.

use url::Url;

/// Parse an OIDC endpoint without weakening its transport or authority.
///
/// Preconditions: `raw` is an absolute, credential-free HTTPS URL, or an HTTP
/// URL whose host is an explicit loopback name/address. Fragments are forbidden
/// because they are not sent in HTTP requests and can conceal configuration
/// mistakes. Query parameters are preserved because OAuth metadata endpoints
/// may legitimately contain them.
///
/// Postcondition: the returned URL has the same scheme, authority, path and
/// query as `raw` and is safe to use as a credential-bearing request target.
pub(crate) fn secure_endpoint(raw: &str) -> Result<Url, ()> {
    let url = Url::parse(raw).map_err(|_| ())?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if url.host_str().is_none()
        || !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(());
    }
    Ok(url)
}

/// Apply [`secure_endpoint`] and additionally reject issuer queries.
///
/// Issuer queries are not part of the normalized issuer identity and must not
/// be silently discarded before discovery issuer comparison.
pub(crate) fn secure_issuer(raw: &str) -> Result<Url, ()> {
    let url = secure_endpoint(raw)?;
    if url.query().is_some() {
        return Err(());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_policy_preserves_queries_and_rejects_ambiguous_authorities() {
        for valid in [
            "https://issuer.example/token?tenant=one",
            "http://127.0.0.1/token",
            "http://localhost/token",
            "http://[::1]/token",
        ] {
            assert_eq!(secure_endpoint(valid).unwrap().as_str(), valid);
        }
        for invalid in [
            "relative",
            "http://issuer.example/token",
            "file:///tmp/token",
            "https://user@issuer.example/token",
            "https://user:password@issuer.example/token",
            "https://issuer.example/token#fragment",
        ] {
            assert!(secure_endpoint(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn issuer_policy_rejects_metadata_that_normalization_would_discard() {
        assert!(secure_issuer("https://issuer.example/realm").is_ok());
        for invalid in [
            "https://issuer.example/realm?tenant=one",
            "https://issuer.example/realm#fragment",
            "https://user@issuer.example/realm",
        ] {
            assert!(secure_issuer(invalid).is_err(), "{invalid}");
        }
    }
}
