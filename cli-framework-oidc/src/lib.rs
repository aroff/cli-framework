//! OIDC/OAuth2 provider for cli-framework.
//!
//! Two independent features:
//! - `client`: [`OidcClient`] implementing [`cli_framework::auth::TokenProvider`]
//! - `server`: [`server::oidc_validation_layer`] tower middleware + [`server::OidcClaims`] extractor

/// Shared error type used by both `client` and `server` features.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum OidcConfigError {
    #[error("issuer_url must be absolute https (or http://127.0.0.1 | http://localhost for local dev): {0}")]
    InsecureIssuer(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("algorithms must be non-empty")]
    EmptyAlgorithms,
    #[error("invalid jwks_uri: {0}")]
    InvalidJwksUri(String),
    #[error("invalid flow configuration: {0}")]
    InvalidFlow(String),
}

/// Validate that a JWKS URI is secure: must be https, or http to loopback only.
#[cfg(feature = "server")]
pub(crate) fn validate_jwks_uri(uri: &str) -> Result<(), OidcConfigError> {
    let url =
        url::Url::parse(uri).map_err(|e| OidcConfigError::InvalidJwksUri(format!("{uri}: {e}")))?;
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("");
    let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "[::1]";
    if scheme == "http" && !is_loopback {
        return Err(OidcConfigError::InvalidJwksUri(format!(
            "insecure JWKS URI (non-loopback http): {uri}"
        )));
    }
    if scheme != "https" && !(scheme == "http" && is_loopback) {
        return Err(OidcConfigError::InvalidJwksUri(format!(
            "unsupported scheme in JWKS URI: {uri}"
        )));
    }
    Ok(())
}

/// Normalize and validate an OIDC issuer URL.
///
/// - Requires https (or http://127.0.0.1, http://localhost, http://[::1] for local dev).
/// - Lowercases scheme and host.
/// - Strips default ports (443 for https, 80 for http).
/// - Strips trailing slash.
pub fn normalize_issuer(raw: &str) -> Result<String, OidcConfigError> {
    let url =
        url::Url::parse(raw).map_err(|e| OidcConfigError::InsecureIssuer(format!("{raw}: {e}")))?;
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("");

    let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "[::1]";
    if scheme == "http" && !is_loopback {
        return Err(OidcConfigError::InsecureIssuer(raw.to_string()));
    }
    if scheme != "https" && !(scheme == "http" && is_loopback) {
        return Err(OidcConfigError::InsecureIssuer(raw.to_string()));
    }

    let scheme = scheme.to_lowercase();
    let host = host.to_lowercase();
    let port = url.port();

    let include_port = match port {
        Some(443) if scheme == "https" => false,
        Some(80) if scheme == "http" => false,
        Some(_) => true,
        None => false,
    };

    let authority = if include_port {
        format!("{}:{}", host, port.unwrap())
    } else {
        host
    };

    let path = url.path().trim_end_matches('/');

    if path.is_empty() || path == "/" {
        Ok(format!("{}://{}", scheme, authority))
    } else {
        Ok(format!("{}://{}{}", scheme, authority, path))
    }
}

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "server")]
pub mod server;
