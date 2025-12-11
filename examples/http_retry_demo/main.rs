//! Example demonstrating HTTP retry integration usage
//!
//! This example shows how to use RetryableHttpClient to make HTTP requests
//! with automatic retry logic.

use reqwest::Client;
use std::time::Duration;
use tui_framework::http_retry::RetryableHttpClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("HTTP Retry Demo");
    println!("===============\n");

    // Create a retryable HTTP client with default policy
    // Default: 3 retries, 1s initial delay, 10s max delay (exponential backoff)
    let client = Client::new();
    let retry_client = RetryableHttpClient::new(client);

    // Example 1: Simple GET request with automatic retry
    println!("Example 1: Simple GET request");
    match retry_client.get("https://httpbin.org/status/200").await {
        Ok(response) => {
            println!("  ✓ Request succeeded: {}", response.status());
        }
        Err(e) => {
            println!("  ✗ Request failed: {}", e);
        }
    }

    // Example 2: GET request that will retry on 5xx errors
    println!("\nExample 2: GET request with retry on server errors");
    match retry_client.get("https://httpbin.org/status/500").await {
        Ok(response) => {
            println!("  ✓ Request succeeded after retries: {}", response.status());
        }
        Err(e) => {
            println!("  ✗ Request failed after retries: {}", e);
        }
    }

    // Example 3: POST request with custom headers and body
    println!("\nExample 3: POST request with custom configuration");
    let result = retry_client
        .execute_with_retry(|| {
            retry_client
                .client()
                .post("https://httpbin.org/post")
                .header("X-Custom-Header", "demo-value")
                .json(&serde_json::json!({"message": "Hello from retry client"}))
        })
        .await;

    match result {
        Ok(response) => {
            println!("  ✓ POST request succeeded: {}", response.status());
        }
        Err(e) => {
            println!("  ✗ POST request failed: {}", e);
        }
    }

    // Example 4: Using custom error classifier
    println!("\nExample 4: Custom error classifier");
    let result = retry_client
        .execute_with_classifier(
            || retry_client.client().get("https://httpbin.org/status/503"),
            |error| {
                // Custom logic: only retry on 503, not other 5xx
                if let Some(status) = error.status() {
                    status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                } else {
                    // Retry on network errors
                    error.is_timeout() || error.is_connect()
                }
            },
        )
        .await;

    match result {
        Ok(response) => {
            println!(
                "  ✓ Request with custom classifier succeeded: {}",
                response.status()
            );
        }
        Err(e) => {
            println!("  ✗ Request with custom classifier failed: {}", e);
        }
    }

    // Example 5: Custom retry policy
    println!("\nExample 5: Custom retry policy");
    use tui_framework::retry::RetryPolicy;

    let custom_policy = RetryPolicy::fixed_delay(5, Duration::from_millis(200));
    let client_with_policy = RetryableHttpClient::with_policy(Client::new(), custom_policy);

    match client_with_policy
        .get("https://httpbin.org/status/200")
        .await
    {
        Ok(response) => {
            println!(
                "  ✓ Request with custom policy succeeded: {}",
                response.status()
            );
        }
        Err(e) => {
            println!("  ✗ Request with custom policy failed: {}", e);
        }
    }

    // Example 6: Per-request policy override
    println!("\nExample 6: Per-request policy override");
    let default_client = RetryableHttpClient::new(Client::new());
    let per_request_policy =
        RetryPolicy::exponential_backoff(2, Duration::from_millis(100), Duration::from_secs(2));

    let result = default_client
        .execute_with_policy(
            || {
                default_client
                    .client()
                    .get("https://httpbin.org/status/200")
            },
            per_request_policy,
        )
        .await;

    match result {
        Ok(response) => {
            println!(
                "  ✓ Request with per-request policy succeeded: {}",
                response.status()
            );
        }
        Err(e) => {
            println!("  ✗ Request with per-request policy failed: {}", e);
        }
    }

    // Example 7: Smart error classification demonstration
    println!("\nExample 7: Smart error classification");
    println!("  Demonstrating that 4xx errors (404, 401) are NOT retried");
    println!("  while 5xx errors and network errors ARE retried");

    // This will fail immediately without retries (404 is not retryable)
    match retry_client.get("https://httpbin.org/status/404").await {
        Ok(_) => println!("  ✗ Unexpected: 404 should not succeed"),
        Err(e) => println!("  ✓ 404 correctly failed without retries: {}", e),
    }

    // This will retry and eventually fail (500 is retryable but will keep failing)
    match retry_client.get("https://httpbin.org/status/500").await {
        Ok(_) => println!("  ✗ Unexpected: 500 should not succeed"),
        Err(e) => println!("  ✓ 500 correctly retried then failed: {}", e),
    }

    // Example 8: Custom error classification
    println!("\nExample 8: Custom error classification");
    println!("  Demonstrating custom classifier that marks 404 as retryable");

    let result = retry_client
        .execute_with_classifier(
            || retry_client.client().get("https://httpbin.org/status/404"),
            |error| {
                // Custom logic: retry on 404 (normally not retryable)
                if let Some(status) = error.status() {
                    status == reqwest::StatusCode::NOT_FOUND
                        || status.is_server_error()
                        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                } else {
                    error.is_timeout() || error.is_connect()
                }
            },
        )
        .await;

    match result {
        Ok(_) => {
            println!("  ✓ Custom classifier allowed 404 retry (would succeed if server recovered)")
        }
        Err(e) => println!("  ✗ Custom classifier test failed: {}", e),
    }

    // Example 9: Retry-After header support
    println!("\nExample 9: Retry-After header support");
    println!("  Demonstrating automatic respect for Retry-After headers in 429 responses");
    println!("  Note: This requires a server that returns 429 with Retry-After header");
    println!("  The client will automatically wait for the specified duration before retrying");

    // Example 10: Logging configuration
    println!("\nExample 10: Logging configuration");
    println!("  The HTTP retry client uses standard Rust logging (log crate)");
    println!("  To enable logging, set the RUST_LOG environment variable:");
    println!("    RUST_LOG=debug cargo run --example http_retry_demo");
    println!("    RUST_LOG=tui_framework::http_retry=debug cargo run --example http_retry_demo");
    println!("  Log levels:");
    println!("    - debug: Retry attempts, delays, and detailed information");
    println!("    - info: Retry completion (when request succeeds after retries)");
    println!("    - warn: Retry exhaustion (when all attempts fail)");

    println!("\nDemo completed!");
    Ok(())
}
