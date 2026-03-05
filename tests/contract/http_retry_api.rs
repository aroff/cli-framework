//! Contract tests for HTTP retry API
//!
//! These tests verify that HTTP error classification functions comply with the API contract
//! as defined in the specification.

use reqwest::Error;
use cli_framework::http_retry::http_errors::is_retryable_http_error;

#[tokio::test]
async fn test_is_retryable_http_error_with_network_errors() {
    // T005: Contract test for network errors
    // Network errors (connection failures, timeouts) should be retryable
    
    // Create a connection error by trying to connect to an invalid address
    let client = reqwest::Client::new();
    let result = client
        .get("http://127.0.0.1:1") // Invalid port, will cause connection error
        .timeout(std::time::Duration::from_millis(100))
        .send()
        .await;
    
    if let Err(e) = result {
        assert!(
            is_retryable_http_error(&e),
            "Network connection errors should be retryable"
        );
    } else {
        panic!("Expected connection error, but request succeeded");
    }
}

#[tokio::test]
async fn test_is_retryable_http_error_with_5xx_server_errors() {
    // T006: Contract test for 5xx server errors
    // 5xx server errors should be retryable
    
    // Use wiremock to simulate 5xx errors
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;
    
    let mock_server = MockServer::start().await;
    
    // Mock 500 Internal Server Error
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;
    
    let client = reqwest::Client::new();
    let result = client.get(&mock_server.uri()).send().await;
    
    // reqwest returns Ok(response) even for error status codes
    // We need to call error_for_status() to convert it to an Error
    match result {
        Ok(response) => {
            assert_eq!(response.status().as_u16(), 500);
            // Convert response with error status to Error
            let error = response.error_for_status().unwrap_err();
            assert!(
                is_retryable_http_error(&error),
                "5xx server errors should be retryable"
            );
        }
        Err(e) => {
            // If we got an error directly, it should also be retryable
            assert!(
                is_retryable_http_error(&e),
                "5xx server errors should be retryable"
            );
        }
    }
}

#[tokio::test]
async fn test_is_retryable_http_error_with_429_rate_limit_errors() {
    // T007: Contract test for 429 rate limit errors
    // 429 Too Many Requests should be retryable
    
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;
    
    let mock_server = MockServer::start().await;
    
    // Mock 429 Too Many Requests
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&mock_server)
        .await;
    
    let client = reqwest::Client::new();
    let result = client.get(&mock_server.uri()).send().await;
    
    match result {
        Ok(response) => {
            assert_eq!(response.status().as_u16(), 429);
            // Convert response with error status to Error
            let error = response.error_for_status().unwrap_err();
            assert!(
                is_retryable_http_error(&error),
                "429 rate limit errors should be retryable"
            );
        }
        Err(e) => {
            panic!("Expected 429 response, but got error: {:?}", e);
        }
    }
}

#[tokio::test]
async fn test_is_retryable_http_error_with_4xx_client_errors() {
    // T008: Contract test for 4xx client errors (should return false)
    // 4xx client errors (except 429) should NOT be retryable
    
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;
    
    let mock_server = MockServer::start().await;
    
    // Mock 400 Bad Request
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&mock_server)
        .await;
    
    let client = reqwest::Client::new();
    let result = client.get(&mock_server.uri()).send().await;
    
    match result {
        Ok(response) => {
            assert_eq!(response.status().as_u16(), 400);
            // Convert response with error status to Error
            let error = response.error_for_status().unwrap_err();
            assert!(
                !is_retryable_http_error(&error),
                "4xx client errors (except 429) should NOT be retryable"
            );
        }
        Err(e) => {
            panic!("Expected 400 response, but got error: {:?}", e);
        }
    }
    
    // Also test 404 Not Found
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;
    
    let result = client.get(&mock_server.uri()).send().await;
    match result {
        Ok(response) => {
            assert_eq!(response.status().as_u16(), 404);
            let error = response.error_for_status().unwrap_err();
            assert!(
                !is_retryable_http_error(&error),
                "404 Not Found should NOT be retryable"
            );
        }
        Err(e) => {
            panic!("Expected 404 response, but got error: {:?}", e);
        }
    }
}
