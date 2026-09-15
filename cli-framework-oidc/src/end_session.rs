//! Native RP-initiated logout without transferring browser or credential custody.
//!
//! Preparing a URL neither clears local credentials nor proves SSO termination.
//! Hosts independently clear credentials and launch/report this URL. Without an
//! ID-token hint, the provider may require browser confirmation before logout.

use crate::client::OidcClient;
use cli_framework::auth::AuthError;
use std::time::Duration;
use url::Url;

impl OidcClient {
    /// Prepare the provider's advertised end-session URL for this public client.
    ///
    /// Preconditions: optional redirect is a registered absolute URI; state is
    /// nonempty and supplied only with a redirect. The host must validate that
    /// state if it handles the return callback. Postcondition: the result names
    /// the discovered endpoint with exactly this client ID and optional return
    /// parameters. No bearer/refresh token is sent, no browser is opened, and
    /// local credentials are unchanged even when discovery fails.
    pub async fn end_session_url(
        &self,
        post_logout_redirect_uri: Option<&str>,
        state: Option<&str>,
    ) -> Result<String, AuthError> {
        validate_return(post_logout_redirect_uri, state)?;
        let issuer = secure_endpoint(self.issuer_url())?;
        if issuer.query().is_some() || issuer.fragment().is_some() {
            return Err(failure("issuer must not contain a query or fragment"));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| failure("could not initialize OIDC discovery transport"))?;
        let response = http
            .get(format!(
                "{}/.well-known/openid-configuration",
                self.issuer_url()
            ))
            .send()
            .await
            .map_err(|_| failure("end-session discovery request failed"))?;
        if !response.status().is_success() {
            return Err(failure(
                "end-session discovery returned a non-success status",
            ));
        }
        let document: serde_json::Value = response
            .json()
            .await
            .map_err(|_| failure("end-session discovery returned invalid JSON"))?;
        let discovered_issuer = document["issuer"].as_str().unwrap_or_default();
        if crate::normalize_issuer(discovered_issuer).ok().as_deref() != Some(self.issuer_url()) {
            return Err(failure("end-session discovery issuer mismatch"));
        }
        let endpoint = document["end_session_endpoint"]
            .as_str()
            .ok_or(AuthError::NotSupported(
                "issuer does not advertise end-session",
            ))?;
        compose_url(endpoint, self.client_id(), post_logout_redirect_uri, state)
    }
}

/// Accept HTTPS or development loopback HTTP only, with no credential/fragment.
fn secure_endpoint(raw: &str) -> Result<Url, AuthError> {
    let url = Url::parse(raw).map_err(|_| failure("invalid OIDC endpoint URL"))?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if url.host_str().is_none()
        || !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(failure(
            "OIDC endpoint must be credential-free HTTPS or loopback HTTP",
        ));
    }
    Ok(url)
}

/// Validate callback arguments before networking; no arbitrary browser script URIs.
fn validate_return(redirect: Option<&str>, state: Option<&str>) -> Result<(), AuthError> {
    if state.is_some_and(str::is_empty) || state.is_some() && redirect.is_none() {
        return Err(failure(
            "logout state requires a return URI and must be nonempty",
        ));
    }
    if let Some(raw) = redirect {
        let url = Url::parse(raw).map_err(|_| failure("invalid logout return URI"))?;
        if matches!(url.scheme(), "javascript" | "data" | "file")
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(failure("unsafe logout return URI"));
        }
        if matches!(url.scheme(), "http" | "https") {
            secure_endpoint(raw)?;
        }
    }
    Ok(())
}

/// Produce a unique parameter binding; reject metadata that pre-binds RP parameters.
fn compose_url(
    endpoint: &str,
    client_id: &str,
    redirect: Option<&str>,
    state: Option<&str>,
) -> Result<String, AuthError> {
    let mut url = secure_endpoint(endpoint)?;
    if url.query_pairs().any(|(key, _)| {
        matches!(
            key.as_ref(),
            "client_id" | "post_logout_redirect_uri" | "state" | "id_token_hint"
        )
    }) {
        return Err(failure(
            "discovered logout endpoint pre-binds reserved parameters",
        ));
    }
    url.query_pairs_mut().append_pair("client_id", client_id);
    if let Some(redirect) = redirect {
        url.query_pairs_mut()
            .append_pair("post_logout_redirect_uri", redirect);
    }
    if let Some(state) = state {
        url.query_pairs_mut().append_pair("state", state);
    }
    Ok(url.into())
}

/// Keep provider errors actionable without embedding token-bearing URLs or bodies.
fn failure(message: &str) -> AuthError {
    AuthError::Provider {
        message: message.to_owned(),
        source: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::OidcFlow;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[test]
    fn endpoint_security_is_closed_except_explicit_loopback() {
        for valid in [
            "https://issuer.example/logout",
            "http://127.0.0.1/logout",
            "http://localhost/logout",
            "http://[::1]/logout",
        ] {
            assert!(secure_endpoint(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "relative",
            "http://issuer.example/logout",
            "file:///tmp/a",
            "https://user:password@issuer.example/",
            "https://issuer.example/#fragment",
        ] {
            assert!(secure_endpoint(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn callback_arguments_are_validated_before_networking() {
        assert!(validate_return(None, None).is_ok());
        assert!(validate_return(Some("com.example.app:/logout"), Some("csrf-state")).is_ok());
        assert!(validate_return(Some("https://app.example/logout"), None).is_ok());
        assert!(validate_return(Some("http://127.0.0.1:8765/logout"), None).is_ok());
        for (uri, state) in [
            (None, Some("state")),
            (Some("https://a.test"), Some("")),
            (Some("relative"), None),
            (Some("javascript:alert(1)"), None),
            (Some("data:text/plain,a"), None),
            (Some("file:///tmp/a"), None),
            (Some("https://a.test/#x"), None),
            (Some("http://remote.test/logout"), None),
            (Some("https://u:p@a.test/logout"), None),
        ] {
            assert!(validate_return(uri, state).is_err());
        }
    }

    #[test]
    fn composition_encodes_parameters_and_preserves_unrelated_metadata() {
        let raw = compose_url(
            "https://id.example/logout?tenant=one",
            "client & two",
            Some("com.example.app:/logout"),
            Some("state?&"),
        )
        .unwrap();
        let url = Url::parse(&raw).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.len(), 4);
        assert_eq!(query["client_id"], "client & two");
        assert_eq!(query["post_logout_redirect_uri"], "com.example.app:/logout");
        assert_eq!(query["state"], "state?&");
        assert_eq!(query["tenant"], "one");
        assert_eq!(
            compose_url("https://id.example/logout", "client", None, None).unwrap(),
            "https://id.example/logout?client_id=client"
        );
        for reserved in [
            "client_id",
            "post_logout_redirect_uri",
            "state",
            "id_token_hint",
        ] {
            assert!(compose_url(
                &format!("https://id.example/logout?{reserved}=injected"),
                "client",
                None,
                None
            )
            .is_err());
        }
        assert!(compose_url("http://remote.test/logout", "client", None, None).is_err());
    }

    #[tokio::test]
    async fn real_discovery_is_issuer_bound_without_credentials() {
        let server = MockServer::start().await;
        let temp = tempfile::tempdir().unwrap();
        let client = OidcClient::builder()
            .issuer_url(server.uri())
            .client_id("native-test")
            .flow(OidcFlow::AuthCodePkce {
                redirect: Default::default(),
            })
            .cache_dir(temp.path().into())
            .build()
            .unwrap();
        Mock::given(method("GET")).and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"issuer":server.uri(), "end_session_endpoint":format!("{}/logout",server.uri())})))
            .expect(1).mount(&server).await;
        assert_eq!(
            client.end_session_url(None, None).await.unwrap(),
            format!("{}/logout?client_id=native-test", server.uri())
        );
        let requests = server.received_requests().await.unwrap();
        assert!(!requests[0].headers.contains_key("authorization"));
        assert!(!requests[0].headers.contains_key("cookie"));
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn discovery_errors_are_not_successful_logout() {
        for (status, body, unsupported) in [
            (404, "{}", false),
            (200, "not json", false),
            (200, "{\"issuer\":\"https://wrong.test\"}", false),
            (302, "{}", false),
            (200, "{}", false),
            (200, "MATCHING_ISSUER", true),
        ] {
            let server = MockServer::start().await;
            let temp = tempfile::tempdir().unwrap();
            let client = OidcClient::builder()
                .issuer_url(server.uri())
                .client_id("native-test")
                .flow(OidcFlow::AuthCodePkce {
                    redirect: Default::default(),
                })
                .cache_dir(temp.path().into())
                .build()
                .unwrap();
            let response = if body == "MATCHING_ISSUER" {
                ResponseTemplate::new(status)
                    .set_body_json(serde_json::json!({"issuer":server.uri()}))
            } else {
                ResponseTemplate::new(status).set_body_string(body)
            };
            Mock::given(method("GET"))
                .respond_with(response)
                .mount(&server)
                .await;
            let error = client.end_session_url(None, None).await.unwrap_err();
            assert_eq!(matches!(error, AuthError::NotSupported(_)), unsupported);
        }
        let temp = tempfile::tempdir().unwrap();
        let client = OidcClient::builder()
            .issuer_url("http://127.0.0.1:9")
            .client_id("native-test")
            .flow(OidcFlow::AuthCodePkce {
                redirect: Default::default(),
            })
            .cache_dir(temp.path().into())
            .build()
            .unwrap();
        assert!(client.end_session_url(None, None).await.is_err());
        assert!(client
            .end_session_url(None, Some("invalid-state"))
            .await
            .is_err());
    }
}
