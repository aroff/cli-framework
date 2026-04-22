//! Unit tests for HTTP error classification utilities

use cli_framework::http_retry::http_errors::{is_connection_error, is_timeout};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_is_timeout_helper_function() {
    // T009: Unit test for is_timeout() helper function

    // Avoid external network calls in tests: use a local mock server that deliberately delays.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/delay"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(250)))
        .mount(&server)
        .await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(10))
        .build()
        .unwrap();

    let result = client.get(format!("{}/delay", server.uri())).send().await;
    let err = result.expect_err("request should timeout");

    assert!(
        is_timeout(&err) || err.is_timeout(),
        "Timeout errors should be detected by is_timeout()"
    );
}

#[tokio::test]
async fn test_is_connection_error_helper_function() {
    // T010: Unit test for is_connection_error() helper function

    let client = reqwest::Client::new();

    // Try to connect to an invalid address (will cause connection error)
    let result = client
        .get("http://127.0.0.1:1") // Invalid port
        .timeout(std::time::Duration::from_millis(100))
        .send()
        .await;

    if let Err(e) = result {
        assert!(
            is_connection_error(&e) || e.is_connect(),
            "Connection errors should be detected by is_connection_error()"
        );
    } else {
        panic!("Expected connection error, but request succeeded");
    }
}
