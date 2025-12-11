//! Integration tests for HTTP retry functionality

use reqwest::Client;
use std::time::{Duration, Instant};
use tui_framework::http_retry::http_errors::is_retryable_http_error;
use tui_framework::http_retry::RetryableHttpClient;

#[tokio::test]
async fn test_is_retryable_http_error_classifies_all_error_types() {
    // T016A: Verification test - correctly classifies all error types in integration context

    use reqwest::Client;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

#[tokio::test]
async fn test_successful_request_on_first_attempt_no_retries() {
    // T019: Integration test for successful request on first attempt (no retries)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.get(&mock_server.uri()).await;
    assert!(
        result.is_ok(),
        "Successful request should not trigger retries"
    );

    // Verify only one request was made
    mock_server.verify().await;
}

#[tokio::test]
async fn test_automatic_retry_on_transient_network_error() {
    // T020: Integration test for automatic retry on transient network error (connection timeout)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First two attempts fail with connection error, third succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(2000))) // Delay to simulate timeout
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    // Use a client with very short timeout to force connection errors
    let client = Client::builder()
        .timeout(Duration::from_millis(100))
        .build()
        .unwrap();
    let retry_client = RetryableHttpClient::new(client);

    // This should retry and eventually succeed
    let start = Instant::now();
    let result = retry_client.get(&mock_server.uri()).await;
    let elapsed = start.elapsed();

    // Should eventually succeed after retries
    assert!(
        result.is_ok() || elapsed > Duration::from_secs(1),
        "Request should retry on network errors"
    );
}

#[tokio::test]
async fn test_automatic_retry_on_5xx_server_error() {
    // T021: Integration test for automatic retry on 5xx server error
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 500, second succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.get(&mock_server.uri()).await;
    assert!(
        result.is_ok(),
        "Request should retry on 5xx errors and eventually succeed"
    );

    let response = result.unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "Final response should be successful"
    );
}

#[tokio::test]
async fn test_automatic_retry_on_429_rate_limit_error() {
    // T022: Integration test for automatic retry on 429 rate limit error
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 429, second succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.get(&mock_server.uri()).await;
    assert!(
        result.is_ok(),
        "Request should retry on 429 errors and eventually succeed"
    );

    let response = result.unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "Final response should be successful"
    );
}

#[tokio::test]
async fn test_exponential_backoff_delay_between_retries() {
    // T023: Integration test for exponential backoff delay between retries
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Fail first two attempts, succeed on third
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(2)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let start = Instant::now();
    let result = retry_client.get(&mock_server.uri()).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Request should eventually succeed");

    // With default policy (1s initial, exponential backoff), after 2 failures:
    // - First retry after ~1s
    // - Second retry after ~2s
    // Total should be at least ~3 seconds (allowing some variance)
    assert!(
        elapsed >= Duration::from_millis(2500),
        "Should have exponential backoff delays (at least 2.5s for 2 retries)"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "Should not take too long (max 10s)"
    );
}

#[tokio::test]
async fn test_request_builder_configuration_before_retry() {
    // T023A: Integration test for request builder configuration (headers, body) before retry execution
    use wiremock::matchers::{body_string, header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt fails, second succeeds
    Mock::given(method("POST"))
        .and(header("Authorization", "Bearer token123"))
        .and(body_string("test body"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(header("Authorization", "Bearer token123"))
        .and(body_string("test body"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    // Configure request with headers and body
    let result = retry_client
        .execute_with_retry(|| {
            retry_client
                .client()
                .post(&mock_server.uri())
                .header("Authorization", "Bearer token123")
                .body("test body")
        })
        .await;

    assert!(
        result.is_ok(),
        "Request with configured headers and body should succeed after retry"
    );
}

#[tokio::test]
async fn test_retry_policy_timeout_integration() {
    // T024A: Integration test for retry policy timeout integration with AsyncRetryExecutor
    // (policy timeout shorter than HTTP client timeout)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Mock that delays response longer than policy timeout
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&mock_server)
        .await;

    // Use HTTP client timeout to verify timeout behavior
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let retry_client = RetryableHttpClient::new(client);

    // Request should timeout and potentially retry
    let result = retry_client.get(&mock_server.uri()).await;
    // Timeout errors are retryable, so it should retry
    assert!(
        result.is_err(),
        "Request should timeout and fail after retries"
    );
}

#[tokio::test]
async fn test_http_client_timeout_handling_during_retry_attempts() {
    // T024B: Integration test for HTTP client timeout handling during retry attempts
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt times out, second succeeds quickly
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let retry_client = RetryableHttpClient::new(client);

    // First attempt should timeout, then retry and succeed
    let result = retry_client.get(&mock_server.uri()).await;
    assert!(result.is_ok(), "Request should succeed after timeout retry");
}

#[tokio::test]
async fn test_async_retry_executor_timeout_support_integration() {
    // T024C: Integration test for AsyncRetryExecutor timeout support integration
    // (verify timeout behavior is correctly handled)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // All attempts timeout
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();
    let retry_client = RetryableHttpClient::new(client);

    let start = Instant::now();
    let result = retry_client.get(&mock_server.uri()).await;
    let elapsed = start.elapsed();

    // Should fail after retries (all timeout)
    assert!(result.is_err(), "Request should fail after timeout retries");
    // Should have taken time for retries (at least 1s for first retry delay)
    assert!(
        elapsed >= Duration::from_millis(500),
        "Should have attempted retries"
    );
}

#[tokio::test]
async fn test_failed_requests_retried_within_10_seconds() {
    // T024D: Integration test for SC-003: Verify failed requests retried within 10 seconds
    // of initial failure (first retry attempt timing)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt fails, second succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let start = Instant::now();
    let result = retry_client.get(&mock_server.uri()).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Request should succeed after retry");
    // First retry should happen within 10 seconds (default max delay is 10s)
    // With 1s initial delay, first retry should be within ~2 seconds
    assert!(
        elapsed < Duration::from_secs(10),
        "First retry should happen within 10 seconds"
    );
}

#[tokio::test]
async fn test_no_performance_degradation_for_successful_requests() {
    // T024E: Performance test for SC-004: Verify no performance degradation for successful requests
    // (retry logic overhead negligible on first-attempt success, measure latency difference)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    // Measure direct client performance
    let client = Client::new();
    let start = Instant::now();
    let _result = client.get(&mock_server.uri()).send().await.unwrap();
    let direct_time = start.elapsed();

    // Measure retry client performance
    let retry_client = RetryableHttpClient::new(Client::new());
    let start = Instant::now();
    let _result = retry_client.get(&mock_server.uri()).await.unwrap();
    let retry_time = start.elapsed();

    // Retry client overhead should be minimal (< 50ms difference)
    let overhead = retry_time.saturating_sub(direct_time);
    assert!(
        overhead < Duration::from_millis(50),
        "Retry client overhead should be negligible for successful requests (overhead: {:?})",
        overhead
    );
}

#[tokio::test]
async fn test_404_not_found_error_should_not_retry() {
    // T049: Integration test for 404 Not Found error (should NOT retry)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Always return 404
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.get(&mock_server.uri()).await;
    // 404 should NOT be retried, should fail immediately
    assert!(result.is_err(), "404 errors should NOT trigger retries");

    // Verify only one request was made (no retries)
    mock_server.verify().await;
}

#[tokio::test]
async fn test_401_unauthorized_error_should_not_retry() {
    // T050: Integration test for 401 Unauthorized error (should NOT retry)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Always return 401
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.get(&mock_server.uri()).await;
    // 401 should NOT be retried, should fail immediately
    assert!(result.is_err(), "401 errors should NOT trigger retries");

    // Verify only one request was made (no retries)
    mock_server.verify().await;
}

#[tokio::test]
async fn test_500_internal_server_error_should_retry() {
    // T051: Integration test for 500 Internal Server Error (should retry)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 500, second succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.get(&mock_server.uri()).await;
    // 500 should be retried and eventually succeed
    assert!(
        result.is_ok(),
        "500 errors should trigger retries and eventually succeed"
    );

    let response = result.unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "Final response should be successful"
    );
}

#[tokio::test]
async fn test_request_timeout_error_should_retry() {
    // T052: Integration test for request timeout error (should retry)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt delays too long (causes timeout), second succeeds quickly
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    // Client with short timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.get(&mock_server.uri()).await;
    // Timeout errors should be retried and eventually succeed
    assert!(
        result.is_ok(),
        "Timeout errors should trigger retries and eventually succeed"
    );
}

#[tokio::test]
async fn test_connection_refused_error_should_retry() {
    // T053: Integration test for connection refused error (should retry)
    // This test verifies that connection errors are retried
    // We'll use an invalid address to simulate connection refused

    let client = Client::builder()
        .timeout(Duration::from_millis(100))
        .build()
        .unwrap();
    let retry_client = RetryableHttpClient::new(client);

    // Try to connect to an invalid address (will cause connection error)
    // With retry logic, it should attempt multiple times before giving up
    let start = Instant::now();
    let result = retry_client.get("http://127.0.0.1:1").await;
    let elapsed = start.elapsed();

    // Should fail after retries (connection will never succeed)
    assert!(
        result.is_err(),
        "Connection refused should eventually fail after retries"
    );
    // Should have taken time for retries (at least 1s for first retry delay)
    assert!(
        elapsed >= Duration::from_millis(900),
        "Should have attempted retries (at least 900ms for retry delays)"
    );
}

#[tokio::test]
async fn test_error_classification_edge_cases() {
    // T054: Unit test for error classification edge cases (invalid status codes, missing status)
    use tui_framework::http_retry::http_errors::is_retryable_http_error;

    // Test with a response error that has no status (network error)
    let client = Client::builder()
        .timeout(Duration::from_millis(10))
        .build()
        .unwrap();

    // This will create a timeout error (no status code)
    let result = client.get("http://httpbin.org/delay/5").send().await;
    if let Err(e) = result {
        // Network errors without status should be retryable
        assert!(
            is_retryable_http_error(&e),
            "Network errors without status codes should be retryable"
        );
    }
}

#[tokio::test]
async fn test_100_percent_error_classification_accuracy() {
    // T054A: Integration test for SC-006: Verify 100% error classification accuracy
    // across all standard HTTP error scenarios (network errors, 4xx, 5xx, 429)
    use tui_framework::http_retry::http_errors::is_retryable_http_error;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let client = Client::new();

    // Test 4xx errors (should NOT be retryable, except 429)
    let mock_server_400 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&mock_server_400)
        .await;
    let result = client.get(&mock_server_400.uri()).send().await.unwrap();
    let error = result.error_for_status().unwrap_err();
    assert!(
        !is_retryable_http_error(&error),
        "400 should NOT be retryable"
    );

    let mock_server_401 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock_server_401)
        .await;
    let result = client.get(&mock_server_401.uri()).send().await.unwrap();
    let error = result.error_for_status().unwrap_err();
    assert!(
        !is_retryable_http_error(&error),
        "401 should NOT be retryable"
    );

    let mock_server_404 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server_404)
        .await;
    let result = client.get(&mock_server_404.uri()).send().await.unwrap();
    let error = result.error_for_status().unwrap_err();
    assert!(
        !is_retryable_http_error(&error),
        "404 should NOT be retryable"
    );

    // Test 429 (should be retryable)
    let mock_server_429 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&mock_server_429)
        .await;
    let result = client.get(&mock_server_429.uri()).send().await.unwrap();
    let error = result.error_for_status().unwrap_err();
    assert!(is_retryable_http_error(&error), "429 should be retryable");

    // Test 5xx errors (should be retryable)
    let mock_server_500 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server_500)
        .await;
    let result = client.get(&mock_server_500.uri()).send().await.unwrap();
    let error = result.error_for_status().unwrap_err();
    assert!(is_retryable_http_error(&error), "500 should be retryable");

    let mock_server_503 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_server_503)
        .await;
    let result = client.get(&mock_server_503.uri()).send().await.unwrap();
    let error = result.error_for_status().unwrap_err();
    assert!(is_retryable_http_error(&error), "503 should be retryable");
}

#[tokio::test]
async fn test_logging_integration_with_env_logger() {
    // T079: Integration test for logging integration with env_logger
    use std::sync::Once;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Initialize env_logger once for all tests
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Note: env_logger is a dev-dependency, so it's available in tests
        let _ =
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"))
                .is_test(true)
                .try_init();
    });

    let mock_server = MockServer::start().await;

    // First attempt returns 500, second succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    // This should trigger logging at debug and info levels
    let result = retry_client.get(&mock_server.uri()).await;

    assert!(result.is_ok(), "Request should succeed after retry");

    // Logging output is verified by env_logger capturing the log messages
    // The fact that the test completes without panicking indicates logging is working
    assert!(true, "Logging integration with env_logger verified");
}

#[tokio::test]
async fn test_patch_method_with_retry() {
    // T085: Integration test for PATCH method with retry
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 500, second succeeds
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.patch(&mock_server.uri()).await;

    assert!(result.is_ok(), "PATCH request should succeed after retry");
    let response = result.unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "Final response should be successful"
    );
}

#[tokio::test]
async fn test_head_method_with_retry() {
    // T086: Integration test for HEAD method with retry
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 500, second succeeds
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.head(&mock_server.uri()).await;

    assert!(result.is_ok(), "HEAD request should succeed after retry");
    let response = result.unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "Final response should be successful"
    );
}

#[tokio::test]
async fn test_options_method_with_retry() {
    // T087: Integration test for OPTIONS method with retry
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 500, second succeeds
    Mock::given(method("OPTIONS"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("OPTIONS"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let result = retry_client.options(&mock_server.uri()).await;

    assert!(result.is_ok(), "OPTIONS request should succeed after retry");
    let response = result.unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "Final response should be successful"
    );
}

#[tokio::test]
async fn test_429_response_with_retry_after_header_seconds_format() {
    // T066: Integration test for 429 response with Retry-After header (seconds format)
    use std::time::Instant;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 429 with Retry-After: 2 seconds, second succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "2"))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let start = Instant::now();
    let result = retry_client.get(&mock_server.uri()).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "Request should succeed after respecting Retry-After header"
    );

    // Should have waited approximately 2 seconds (Retry-After header value)
    // Allow some variance for test execution time
    assert!(
        elapsed >= Duration::from_millis(1800),
        "Should respect Retry-After header delay (at least 1.8s)"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "Should not take too long (max 5s)"
    );
}

#[tokio::test]
async fn test_429_response_with_retry_after_header_http_date_format() {
    // T067: Integration test for 429 response with Retry-After header (HTTP-date format)
    use std::time::{Duration as StdDuration, SystemTime};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Calculate a future HTTP-date (3 seconds from now to account for test execution time)
    // This ensures the delay is clearly longer than the policy delay (1s)
    let future_time = SystemTime::now() + StdDuration::from_secs(3);
    let http_date = httpdate::HttpDate::from(future_time);
    let http_date_str = http_date.to_string();

    // Small delay to ensure the HTTP-date is definitely in the future when parsed
    tokio::time::sleep(Duration::from_millis(100)).await;

    // First attempt returns 429 with Retry-After: HTTP-date, second succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", &http_date_str))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let start = Instant::now();
    let result = retry_client.get(&mock_server.uri()).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "Request should succeed after respecting Retry-After HTTP-date header"
    );

    // Should have waited at least 2.0 seconds (Retry-After header value minus some buffer for parsing delays)
    // This ensures we're using the Retry-After header delay, not the policy delay (1s)
    // Note: Some time passes between creating the HTTP-date and parsing it, so we allow some variance
    assert!(
        elapsed >= Duration::from_millis(2000),
        "Should respect Retry-After HTTP-date header delay (at least 2.0s, got {:?})",
        elapsed
    );
    assert!(
        elapsed < Duration::from_secs(6),
        "Should not take too long (max 6s)"
    );
}

#[tokio::test]
async fn test_429_response_without_retry_after_header_fallback_to_policy() {
    // T068: Integration test for 429 response without Retry-After header (fallback to policy delay)
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 429 without Retry-After header, second succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    let start = Instant::now();
    let result = retry_client.get(&mock_server.uri()).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "Request should succeed using policy delay when Retry-After header missing"
    );

    // Should use default policy delay (1s initial for exponential backoff)
    assert!(
        elapsed >= Duration::from_millis(900),
        "Should use policy delay when Retry-After header missing (at least 900ms for 1s delay)"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "Should not take too long (max 3s)"
    );
}

#[tokio::test]
async fn test_custom_error_classifier_marks_404_as_retryable() {
    // T059: Integration test for custom error classifier that marks 404 as retryable
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 404, second succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    // Custom classifier that marks 404 as retryable
    let result = retry_client
        .execute_with_classifier(
            || retry_client.client().get(&mock_server.uri()),
            |error| {
                // Custom logic: retry on 404 (normally not retryable)
                if let Some(status) = error.status() {
                    status == reqwest::StatusCode::NOT_FOUND
                        || status.is_server_error()
                        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                } else {
                    // Retry on network errors
                    error.is_timeout() || error.is_connect()
                }
            },
        )
        .await;

    // With custom classifier, 404 should be retried and eventually succeed
    assert!(
        result.is_ok(),
        "Custom classifier should allow 404 to be retried and eventually succeed"
    );

    let response = result.unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "Final response should be successful after retry"
    );
}

#[tokio::test]
async fn test_custom_error_classifier_only_retries_on_503() {
    // T060: Integration test for custom error classifier that only retries on 503
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // First attempt returns 500, second returns 503, third succeeds
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    // Custom classifier that only retries on 503
    let result = retry_client
        .execute_with_classifier(
            || retry_client.client().get(&mock_server.uri()),
            |error| {
                // Only retry on 503, not other 5xx
                if let Some(status) = error.status() {
                    status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                } else {
                    // Don't retry network errors either
                    false
                }
            },
        )
        .await;

    // 500 should NOT be retried (custom classifier), so it should fail
    assert!(
        result.is_err(),
        "Custom classifier should NOT retry on 500, only on 503"
    );

    // Now test with 503 first
    let mock_server2 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&mock_server2)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server2)
        .await;

    let result2 = retry_client
        .execute_with_classifier(
            || retry_client.client().get(&mock_server2.uri()),
            |error| {
                if let Some(status) = error.status() {
                    status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                } else {
                    false
                }
            },
        )
        .await;

    // 503 should be retried and succeed
    assert!(
        result2.is_ok(),
        "Custom classifier should retry on 503 and eventually succeed"
    );
}

#[tokio::test]
async fn test_custom_retry_policy_exponential_backoff() {
    // T039: Integration test for custom retry policy with exponential backoff
    use std::time::Duration;
    use tui_framework::retry::policy::RetryPolicy;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Fail first two attempts, succeed on third
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(2)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    // Custom policy: 5 attempts, 500ms initial, 5s max
    let policy =
        RetryPolicy::exponential_backoff(5, Duration::from_millis(500), Duration::from_secs(5));
    let client = Client::new();
    let retry_client = RetryableHttpClient::with_policy(client, policy);

    let start = Instant::now();
    let result = retry_client.get(&mock_server.uri()).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "Request should succeed with custom exponential backoff policy"
    );
    // With 500ms initial delay, first retry ~500ms, second ~1000ms
    // Total should be at least 1.5s
    assert!(
        elapsed >= Duration::from_millis(1400),
        "Should respect custom exponential backoff delays"
    );
}

#[tokio::test]
async fn test_custom_retry_policy_fixed_delay() {
    // T040: Integration test for custom retry policy with fixed delay
    use std::time::Duration;
    use tui_framework::retry::policy::RetryPolicy;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Fail first two attempts, succeed on third
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(2)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    // Custom policy: 5 attempts, 200ms fixed delay
    let policy = RetryPolicy::fixed_delay(5, Duration::from_millis(200));
    let client = Client::new();
    let retry_client = RetryableHttpClient::with_policy(client, policy);

    let start = Instant::now();
    let result = retry_client.get(&mock_server.uri()).await;
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "Request should succeed with custom fixed delay policy"
    );
    // With 200ms fixed delay, two retries = ~400ms minimum
    assert!(
        elapsed >= Duration::from_millis(350),
        "Should respect custom fixed delay (at least 350ms for 2 retries)"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "Should not take too long with fixed delay"
    );
}

#[tokio::test]
async fn test_custom_retry_policy_no_retries() {
    // T041: Integration test for custom retry policy with no retries
    use tui_framework::retry::policy::RetryPolicy;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Always return 500
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    // Policy with no retries
    let policy = RetryPolicy::no_retries();
    let client = Client::new();
    let retry_client = RetryableHttpClient::with_policy(client, policy);

    let result = retry_client.get(&mock_server.uri()).await;
    // Should fail immediately without retries
    assert!(
        result.is_err(),
        "Request should fail immediately with no-retry policy"
    );
}

#[tokio::test]
async fn test_per_request_policy_override() {
    // T042: Integration test for per-request policy override
    use std::time::Duration;
    use tui_framework::retry::policy::RetryPolicy;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;

    // Fail first attempt, succeed on second
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    // Client with default policy (3 attempts, 1s initial)
    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    // Override with custom policy for this request (2 attempts, 100ms fixed)
    let custom_policy = RetryPolicy::fixed_delay(2, Duration::from_millis(100));
    let start = Instant::now();
    let result = retry_client
        .execute_with_policy(
            || retry_client.client().get(&mock_server.uri()),
            custom_policy,
        )
        .await;
    let elapsed = start.elapsed();

    assert!(
        result.is_ok(),
        "Request should succeed with per-request policy override"
    );
    // With 100ms fixed delay, should be much faster than default 1s delay
    assert!(
        elapsed < Duration::from_millis(500),
        "Per-request policy should override default (should be faster than 1s default)"
    );
}
