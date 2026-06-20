//! Tests for normalize_issuer and config validation.

use cli_framework_oidc::{normalize_issuer, OidcConfigError};

#[test]
fn normalize_issuer_strips_trailing_slash() {
    let result = normalize_issuer("https://auth.example.com/").unwrap();
    assert_eq!(result, "https://auth.example.com");
}

#[test]
fn normalize_issuer_strips_default_https_port() {
    let result = normalize_issuer("https://auth.example.com:443/").unwrap();
    assert_eq!(result, "https://auth.example.com");
}

#[test]
fn normalize_issuer_keeps_non_default_port() {
    let result = normalize_issuer("https://auth.example.com:8443/").unwrap();
    assert_eq!(result, "https://auth.example.com:8443");
}

#[test]
fn normalize_issuer_with_path() {
    let result = normalize_issuer("https://auth.example.com/realms/myrealm").unwrap();
    assert_eq!(result, "https://auth.example.com/realms/myrealm");
}

#[test]
fn normalize_issuer_http_loopback_allowed() {
    let result = normalize_issuer("http://127.0.0.1:9000").unwrap();
    assert_eq!(result, "http://127.0.0.1:9000");
}

#[test]
fn normalize_issuer_http_localhost_allowed() {
    let result = normalize_issuer("http://localhost:8080").unwrap();
    assert_eq!(result, "http://localhost:8080");
}

#[test]
fn normalize_issuer_http_public_rejected() {
    let result = normalize_issuer("http://auth.example.com");
    assert!(matches!(result, Err(OidcConfigError::InsecureIssuer(_))));
}

#[test]
fn normalize_issuer_invalid_url() {
    let result = normalize_issuer("not a url");
    assert!(matches!(result, Err(OidcConfigError::InsecureIssuer(_))));
}

#[test]
fn normalize_issuer_lowercases_host() {
    let result = normalize_issuer("https://Auth.Example.COM/realm").unwrap();
    assert_eq!(result, "https://auth.example.com/realm");
}

// Client builder validation tests
#[cfg(feature = "client")]
mod client_validation {
    use cli_framework_oidc::client::{OidcClient, OidcFlow, TokenAuthMethod};
    use cli_framework_oidc::OidcConfigError;
    use secrecy::SecretString;
    use std::path::PathBuf;
    use std::str::FromStr;

    fn make_secret(s: &str) -> SecretString {
        SecretString::from_str(s).unwrap()
    }

    #[test]
    fn builder_missing_issuer_url() {
        let err = OidcClient::builder()
            .client_id("my-client")
            .flow(OidcFlow::ClientCredentials {
                client_secret: make_secret("s3cr3t"),
                token_auth: TokenAuthMethod::Post,
            })
            .cache_dir(PathBuf::from("/tmp"))
            .build()
            .err()
            .expect("should fail");
        assert!(matches!(err, OidcConfigError::MissingField("issuer_url")));
    }

    #[test]
    fn builder_missing_client_id() {
        let err = OidcClient::builder()
            .issuer_url("https://auth.example.com")
            .flow(OidcFlow::ClientCredentials {
                client_secret: make_secret("s3cr3t"),
                token_auth: TokenAuthMethod::Post,
            })
            .cache_dir(PathBuf::from("/tmp"))
            .build()
            .err()
            .expect("should fail");
        assert!(matches!(err, OidcConfigError::MissingField("client_id")));
    }

    #[test]
    fn builder_missing_flow() {
        let err = OidcClient::builder()
            .issuer_url("https://auth.example.com")
            .client_id("my-client")
            .cache_dir(PathBuf::from("/tmp"))
            .build()
            .err()
            .expect("should fail");
        assert!(matches!(err, OidcConfigError::MissingField("flow")));
    }

    #[test]
    fn builder_missing_cache_dir() {
        let err = OidcClient::builder()
            .issuer_url("https://auth.example.com")
            .client_id("my-client")
            .flow(OidcFlow::ClientCredentials {
                client_secret: make_secret("s3cr3t"),
                token_auth: TokenAuthMethod::Post,
            })
            .build()
            .err()
            .expect("should fail");
        assert!(matches!(err, OidcConfigError::MissingField("cache_dir")));
    }

    #[test]
    fn builder_insecure_issuer() {
        let err = OidcClient::builder()
            .issuer_url("http://auth.example.com")
            .client_id("my-client")
            .flow(OidcFlow::ClientCredentials {
                client_secret: make_secret("s3cr3t"),
                token_auth: TokenAuthMethod::Post,
            })
            .cache_dir(PathBuf::from("/tmp"))
            .build()
            .err()
            .expect("should fail");
        assert!(matches!(err, OidcConfigError::InsecureIssuer(_)));
    }
}

// Server config validation tests
#[cfg(feature = "server")]
mod server_validation_config {
    use cli_framework_oidc::server::{oidc_validation_layer, AudiencePolicy, OidcValidationConfig};
    use cli_framework_oidc::OidcConfigError;
    use jsonwebtoken::Algorithm;

    #[test]
    fn empty_algorithms_rejected() {
        let mut cfg = OidcValidationConfig::new(
            "https://auth.example.com",
            AudiencePolicy::Require("my-app".into()),
        );
        cfg.algorithms = vec![];
        let err = oidc_validation_layer(cfg).unwrap_err();
        assert!(matches!(err, OidcConfigError::EmptyAlgorithms));
    }

    #[test]
    fn invalid_jwks_uri_rejected() {
        let mut cfg = OidcValidationConfig::new(
            "https://auth.example.com",
            AudiencePolicy::Require("my-app".into()),
        );
        cfg.jwks_uri = Some("not-a-url".into());
        let err = oidc_validation_layer(cfg).unwrap_err();
        assert!(matches!(err, OidcConfigError::InvalidJwksUri(_)));
    }

    #[test]
    fn insecure_jwks_uri_rejected() {
        let mut cfg = OidcValidationConfig::new(
            "https://auth.example.com",
            AudiencePolicy::Require("my-app".into()),
        );
        cfg.jwks_uri = Some("http://public.example.com/jwks".into());
        let err = oidc_validation_layer(cfg).unwrap_err();
        assert!(matches!(err, OidcConfigError::InvalidJwksUri(_)));
    }

    #[test]
    fn local_http_jwks_uri_allowed() {
        let mut cfg = OidcValidationConfig::new("http://127.0.0.1:9000", AudiencePolicy::Unchecked);
        cfg.algorithms = vec![Algorithm::RS256];
        cfg.jwks_uri = Some("http://127.0.0.1:9000/jwks".into());
        oidc_validation_layer(cfg).expect("should succeed with local http jwks_uri");
    }

    #[test]
    fn valid_config_succeeds() {
        let cfg = OidcValidationConfig::new(
            "https://auth.example.com",
            AudiencePolicy::Require("my-app".into()),
        );
        oidc_validation_layer(cfg).expect("should succeed");
    }
}
