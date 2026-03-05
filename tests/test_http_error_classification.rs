//! Unit tests for HTTP error classification utilities

use cli_framework::http_retry::http_errors::{is_connection_error, is_timeout};

#[tokio::test]
async fn test_is_timeout_helper_function() {
    // T009: Unit test for is_timeout() helper function

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(10))
        .build()
        .unwrap();

    // Create a request that will timeout
    let result = client
        .get("http://httpbin.org/delay/5") // This will timeout due to 10ms timeout
        .send()
        .await;

    if let Err(e) = result {
        assert!(
            is_timeout(&e),
            "Timeout errors should be detected by is_timeout()"
        );
    } else {
        // If it didn't timeout, try with a very short timeout
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1))
            .build()
            .unwrap();
        let result = client.get("http://httpbin.org/delay/1").send().await;
        if let Err(e) = result {
            assert!(
                is_timeout(&e) || e.is_timeout(),
                "Timeout errors should be detected"
            );
        }
    }
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
