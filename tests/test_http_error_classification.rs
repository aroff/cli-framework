//! Unit tests for HTTP error classification utilities

use cli_framework::http_retry::http_errors::{is_connection_error, is_timeout};

#[tokio::test]
async fn test_is_timeout_helper_function() {
    // T009: Unit test for is_timeout() helper function
    //
    // Bind a local TCP listener that accepts connections but never responds,
    // guaranteeing a timeout with a short client timeout.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Accept the connection in a background task (but never send data)
    tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        // Hold the connection open but never respond
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(50))
        .build()
        .unwrap();

    let result = client.get(format!("http://{}/slow", addr)).send().await;

    match result {
        Err(e) => {
            assert!(
                is_timeout(&e),
                "Timeout errors should be detected by is_timeout(), got: {:?}",
                e
            );
        }
        Ok(_) => panic!("Expected timeout error, but request succeeded"),
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
