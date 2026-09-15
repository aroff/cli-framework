//! Bounded, cancellation-safe loopback authorization-code reception.

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Append one set of PKCE parameters to an already security-validated endpoint.
/// Preserve provider query parameters, but reject reserved pre-bound keys.
pub(super) fn authorization_url(
    endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &str,
    state: &str,
    challenge: &str,
) -> Result<String, String> {
    let mut url = url::Url::parse(endpoint).map_err(|_| "invalid authorization endpoint")?;
    let parameters = [
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("scope", scopes),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    if url
        .query_pairs()
        .any(|(key, _)| parameters.iter().any(|(reserved, _)| key == *reserved))
    {
        return Err("authorization endpoint pre-binds reserved OAuth parameters".into());
    }
    url.query_pairs_mut().extend_pairs(parameters);
    Ok(url.into())
}

/// Own all sockets until completion; dropping the future releases them.
/// The deadline covers accept, bounded request reading and response writing.
/// Success proves callback/state validity, not successful token exchange.
pub(super) async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, String> {
    tokio::time::timeout(timeout, async move {
        let (mut stream, _) = listener.accept().await.map_err(|_| "callback accept failed")?;
        let mut line = String::new();
        BufReader::new((&mut stream).take(8193))
            .read_line(&mut line)
            .await
            .map_err(|_| "callback request read failed")?;
        let result = if line.len() > 8192 || !line.ends_with('\n') {
            Err("invalid or oversized callback request".to_owned())
        } else {
            parse_request(&line, expected_state)
        };
        let (status, message) = if result.is_ok() {
            ("200 OK", "Authorization received. Return to the application to finish login.")
        } else {
            ("400 Bad Request", "Authorization callback rejected. Return to the application.")
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{message}",
            message.len()
        );
        stream.write_all(response.as_bytes()).await.map_err(|_| "callback response failed")?;
        result
    })
    .await
    .map_err(|_| "callback timeout".to_owned())?
}

/// Accept exactly one nonempty code and matching state on GET /callback.
/// Provider error descriptions/codes are intentionally not reflected in diagnostics.
fn parse_request(line: &str, expected_state: &str) -> Result<String, String> {
    let parts: Vec<_> = line.split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "GET" || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1") {
        return Err("invalid callback request line".into());
    }
    let (path, query) = parts[1].split_once('?').ok_or("missing callback query")?;
    if path != "/callback" || query.contains('#') {
        return Err("invalid callback path".into());
    }
    let mut codes = Vec::new();
    let mut states = Vec::new();
    let mut provider_error = false;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "code" => codes.push(value.into_owned()),
            "state" => states.push(value.into_owned()),
            "error" => provider_error = true,
            _ => {}
        }
    }
    if expected_state.is_empty() || states.len() != 1 || states[0] != expected_state {
        return Err("state mismatch".into());
    }
    if provider_error {
        return Err("authorization provider rejected login".into());
    }
    if codes.len() != 1 || codes[0].is_empty() {
        return Err("missing or duplicate authorization code".into());
    }
    Ok(codes.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ManualReporter(tokio::sync::mpsc::UnboundedSender<String>);
    impl cli_framework::auth::AuthFlowReporter for ManualReporter {
        fn user_code(&self, _uri: &str, _code: &str) {}
        fn message(&self, message: &str) {
            self.0.send(message.to_owned()).unwrap();
        }
    }

    #[tokio::test]
    async fn manual_pkce_reports_url_and_exchanges_only_the_bound_code() {
        use super::super::{OidcClient, OidcFlow, RedirectConfig, RedirectPort};
        use cli_framework::auth::TokenProvider;
        use std::sync::Arc;
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "issuer": server.uri(), "token_endpoint": format!("{}/token", server.uri()),
                "authorization_endpoint": format!("{}/authorize?tenant=one", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token":"fixture-access", "expires_in":3600, "token_type":"Bearer"
            })))
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let client = OidcClient::builder()
            .issuer_url(server.uri())
            .client_id("native")
            .flow(OidcFlow::AuthCodePkce {
                redirect: RedirectConfig {
                    port: RedirectPort::Ephemeral,
                },
            })
            .cache_dir(directory.path().to_owned())
            .open_browser(false)
            .reporter(Arc::new(ManualReporter(tx)))
            .build()
            .unwrap();
        assert!(!client.open_browser);
        let manual = async {
            let message = rx.recv().await.unwrap();
            let url = url::Url::parse(message.strip_prefix("Open this URL to log in: ").unwrap())
                .unwrap();
            let parameters: std::collections::BTreeMap<_, _> =
                url.query_pairs().into_owned().collect();
            assert_eq!(parameters["tenant"], "one");
            assert_eq!(parameters["client_id"], "native");
            let response = reqwest::Client::new()
                .get(&parameters["redirect_uri"])
                .query(&[
                    ("state", parameters["state"].as_str()),
                    ("code", "fixture-code"),
                ])
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            parameters["code_challenge"].clone()
        };
        let (login, challenge) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(client.login(), manual)
        })
        .await
        .unwrap();
        login.unwrap();
        assert_eq!(client.token().await.unwrap().as_bearer(), "fixture-access");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        let body: std::collections::BTreeMap<_, _> = url::form_urlencoded::parse(&requests[1].body)
            .into_owned()
            .collect();
        assert_eq!(body["grant_type"], "authorization_code");
        assert_eq!(body["code"], "fixture-code");
        assert_eq!(
            crate::pkce::derive_challenge(&body["code_verifier"]),
            challenge
        );
    }

    #[test]
    fn authorization_query_is_preserved_without_ambiguous_oauth_parameters() {
        let result = authorization_url(
            "https://issuer.test/auth?tenant=one",
            "cli +世界",
            "http://127.0.0.1:8765/callback",
            "openid profile",
            "state",
            "challenge",
        )
        .unwrap();
        let url = url::Url::parse(&result).unwrap();
        let query: std::collections::BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.len(), 8);
        assert_eq!(query["tenant"], "one");
        assert_eq!(query["client_id"], "cli +世界");
        assert_eq!(query["code_challenge_method"], "S256");
        assert!(authorization_url("not a URL", "c", "r", "s", "t", "v").is_err());
        for key in [
            "response_type",
            "client_id",
            "redirect_uri",
            "scope",
            "state",
            "code_challenge",
            "code_challenge_method",
            "%73tate",
        ] {
            assert!(authorization_url(
                &format!("https://issuer.test/auth?{key}=x"),
                "c",
                "r",
                "s",
                "t",
                "v"
            )
            .is_err());
        }
    }

    #[test]
    fn callback_parser_requires_exact_path_and_unambiguous_decoded_identity() {
        assert_eq!(
            parse_request(
                "GET /callback?code=a%2Bb&state=s%20t&iss=x HTTP/1.1\r\n",
                "s t"
            )
            .unwrap(),
            "a+b"
        );
        for request in [
            "",
            "POST /callback?code=c&state=s HTTP/1.1",
            "GET /callback?code=c&state=s HTTP/2",
            "GET /callback HTTP/1.1",
            "GET /other?code=c&state=s HTTP/1.1",
            "GET /callback?code=c&state=s#fragment HTTP/1.1",
            "GET /callback?code=c HTTP/1.1",
            "GET /callback?code=c&state=wrong HTTP/1.1",
            "GET /callback?code=c&state=s&%73tate=s HTTP/1.1",
            "GET /callback?state=s HTTP/1.1",
            "GET /callback?code=&state=s HTTP/1.1",
            "GET /callback?code=c&code=c&state=s HTTP/1.1",
            "GET /callback?code=c&state=s&error=sentinel HTTP/1.1",
        ] {
            assert!(parse_request(request, "s").is_err(), "{request}");
        }
        assert!(parse_request("GET /callback?code=c&state= HTTP/1.0", "").is_err());
    }

    #[tokio::test]
    async fn callback_is_bounded_and_returns_truthful_http_status() {
        for (request, succeeds) in [
            ("GET /callback?code=c&state=s HTTP/1.1\r\n".to_owned(), true),
            (
                "GET /callback?code=c&state=wrong HTTP/1.1\r\n".to_owned(),
                false,
            ),
            ("x".repeat(8193), false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let client = async {
                let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                assert!(response.starts_with(if succeeds {
                    "HTTP/1.1 200"
                } else {
                    "HTTP/1.1 400"
                }));
                assert!(!response.contains("Login complete"));
            };
            let (result, ()) = tokio::join!(
                wait_for_callback(listener, "s", Duration::from_secs(2)),
                client
            );
            assert_eq!(result.is_ok(), succeeds);
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        assert_eq!(
            wait_for_callback(listener, "s", Duration::from_millis(10))
                .await
                .unwrap_err(),
            "callback timeout"
        );
        assert!(TcpListener::bind(address).await.is_ok());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let _silent_client = tokio::net::TcpStream::connect(address).await.unwrap();
        assert_eq!(
            wait_for_callback(listener, "s", Duration::from_millis(10))
                .await
                .unwrap_err(),
            "callback timeout"
        );
        assert!(TcpListener::bind(address).await.is_ok());
    }

    #[tokio::test]
    async fn cancellation_releases_listener_without_waiting_for_internal_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        assert!(tokio::time::timeout(
            Duration::from_millis(10),
            wait_for_callback(listener, "s", Duration::from_secs(300))
        )
        .await
        .is_err());
        assert!(TcpListener::bind(address).await.is_ok());
    }
}
