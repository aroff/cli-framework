//! Integration tests for HTTP retry functionality

use cli_framework::http_retry::http_errors::is_retryable_http_error;

#[tokio::test]
async fn test_is_retryable_http_error_classifies_all_error_types() {
    // T016A: Verification test - correctly classifies all error types in integration context
    
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;
    use reqwest::Client;
    
    // Test network errors
    let client = Client::new();
    let network_result = client
        .get("http://127.0.0.1:1")
        .timeout(std::time::Duration::from_millis(100))
        .send()
        .await;
    if let Err(e) = network_result {
        assert!(
            is_retryable_http_error(&e),
            "Network errors should be classified as retryable"
        );
    }
    
    // Test 5xx errors
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;
    
    let result = client.get(&mock_server.uri()).send().await;
    if let Ok(response) = result {
        let error = response.error_for_status().unwrap_err();
        assert!(
            is_retryable_http_error(&error),
            "5xx errors should be classified as retryable"
        );
    }
    
    // Test 429 errors
    let mock_server2 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&mock_server2)
        .await;
    
    let result = client.get(&mock_server2.uri()).send().await;
    if let Ok(response) = result {
        let error = response.error_for_status().unwrap_err();
        assert!(
            is_retryable_http_error(&error),
            "429 errors should be classified as retryable"
        );
    }
    
    // Test 4xx errors (should NOT be retryable)
    let mock_server3 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&mock_server3)
        .await;
    
    let result = client.get(&mock_server3.uri()).send().await;
    if let Ok(response) = result {
        let error = response.error_for_status().unwrap_err();
        assert!(
            !is_retryable_http_error(&error),
            "4xx errors (except 429) should NOT be classified as retryable"
        );
    }
}
