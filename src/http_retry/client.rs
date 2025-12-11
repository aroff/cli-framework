//! Retryable HTTP client implementation
//!
//! Provides a wrapper around `reqwest::Client` that automatically retries
//! failed requests based on configurable retry policies and error classification.

use crate::retry::policy::RetryPolicy;
use anyhow::Result;
use reqwest::{Client, Error as ReqwestError, RequestBuilder, Response};
use std::time::Duration;

use super::http_errors::is_retryable_http_error;

/// HTTP client with automatic retry logic
///
/// Wraps a `reqwest::Client` and provides automatic retry for transient failures.
/// Uses the framework's `AsyncRetryExecutor` and `RetryPolicy` for retry logic.
///
/// # Example
///
/// ```rust,no_run
/// use reqwest::Client;
/// use tui_framework::http_retry::RetryableHttpClient;
///
/// # async fn example() -> anyhow::Result<()> {
/// let client = Client::new();
/// let retry_client = RetryableHttpClient::new(client);
///
/// let response = retry_client.get("https://api.example.com/data").await?;
/// # Ok(())
/// # }
/// ```
pub struct RetryableHttpClient {
    /// The underlying HTTP client
    client: Client,
    /// Default retry policy
    default_policy: RetryPolicy,
}

impl RetryableHttpClient {
    /// Create a new retryable HTTP client with default retry policy
    ///
    /// Default policy: 3 maximum attempts, 1 second initial delay, 10 seconds maximum delay
    /// (exponential backoff)
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use reqwest::Client;
    /// use tui_framework::http_retry::RetryableHttpClient;
    ///
    /// let client = Client::new();
    /// let retry_client = RetryableHttpClient::new(client);
    /// ```
    pub fn new(client: Client) -> Self {
        let default_policy = RetryPolicy::exponential_backoff(
            3,                       // max attempts (3 retries = 4 total attempts)
            Duration::from_secs(1),  // initial delay
            Duration::from_secs(10), // max delay
        );

        Self {
            client,
            default_policy,
        }
    }

    /// Create a new retryable HTTP client with custom retry policy
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use reqwest::Client;
    /// use tui_framework::http_retry::RetryableHttpClient;
    /// use tui_framework::retry::RetryPolicy;
    /// use std::time::Duration;
    ///
    /// let client = Client::new();
    /// let policy = RetryPolicy::exponential_backoff(
    ///     5,  // max attempts
    ///     Duration::from_secs(2),  // initial delay
    ///     Duration::from_secs(30), // max delay
    /// );
    /// let retry_client = RetryableHttpClient::with_policy(client, policy);
    /// ```
    pub fn with_policy(client: Client, policy: RetryPolicy) -> Self {
        Self {
            client,
            default_policy: policy,
        }
    }

    /// Get the underlying reqwest client
    ///
    /// Allows access to the wrapped client for direct operations if needed.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Execute a request builder with retry logic
    ///
    /// This is the core method that handles retry logic. It:
    /// 1. Executes the request
    /// 2. Checks if the error is retryable using the default classifier
    /// 3. Retries using AsyncRetryExecutor if the error is retryable
    /// 4. Returns immediately if the error is not retryable
    ///
    /// # Arguments
    ///
    /// * `request_builder` - A closure that returns a `RequestBuilder`
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use tui_framework::http_retry::RetryableHttpClient;
    /// # use reqwest::Client;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let retry_client = RetryableHttpClient::new(Client::new());
    /// let response = retry_client.execute_with_retry(|| {
    ///     retry_client.client()
    ///         .post("https://api.example.com/data")
    ///         .header("Authorization", "Bearer token")
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn execute_with_retry<F>(&self, request_builder: F) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
    {
        self.execute_with_classifier(request_builder, is_retryable_http_error)
            .await
    }

    /// Execute request with custom error classifier
    ///
    /// Allows custom logic to determine if an error should be retried.
    ///
    /// # Arguments
    ///
    /// * `request_builder` - A closure that returns a `RequestBuilder`
    /// * `classifier` - A function that determines if an error is retryable
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use tui_framework::http_retry::RetryableHttpClient;
    /// # use reqwest::Client;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let retry_client = RetryableHttpClient::new(Client::new());
    /// let response = retry_client.execute_with_classifier(
    ///     || retry_client.client().get("https://api.example.com/data"),
    ///     |error| {
    ///         // Custom logic: retry on 503 but not 500
    ///         if let Some(status) = error.status() {
    ///             status == reqwest::StatusCode::SERVICE_UNAVAILABLE
    ///         } else {
    ///             error.is_timeout() || error.is_connect()
    ///         }
    ///     }
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn execute_with_classifier<F, C>(
        &self,
        request_builder: F,
        classifier: C,
    ) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
        C: Fn(&ReqwestError) -> bool + Send + Sync,
    {
        use tokio::time::sleep;

        let mut last_error: Option<ReqwestError> = None;

        log::debug!(
            "Starting HTTP request with retry (max attempts: {})",
            self.default_policy.max_attempts + 1
        );

        // Try up to max_attempts + 1 times (initial attempt + retries)
        for attempt in 0..=self.default_policy.max_attempts {
            // Build and execute the request
            let builder = request_builder();

            if attempt > 0 {
                log::debug!(
                    "Retry attempt {} of {}",
                    attempt,
                    self.default_policy.max_attempts
                );
            }

            let result = builder.send().await;

            match result {
                Ok(response) => {
                    // Check if response status indicates an error
                    let status = response.status();

                    // Check if this is an error status (4xx or 5xx)
                    if status.is_client_error() || status.is_server_error() {
                        // Extract headers before consuming response
                        let headers = response.headers().clone();

                        // Convert to error and check with classifier
                        let error = response.error_for_status().unwrap_err();
                        last_error = Some(error);

                        // Check if error is retryable using the classifier
                        if !classifier(last_error.as_ref().unwrap()) {
                            // Not retryable, return immediately
                            return Err(anyhow::anyhow!(last_error.unwrap())
                                .context("Non-retryable HTTP error"));
                        }

                        // Error is retryable, but check if we've exhausted retries
                        if attempt >= self.default_policy.max_attempts {
                            // No more retries, return the last error
                            break;
                        }

                        // Check for Retry-After header in 429 responses
                        let delay = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            // Try to parse Retry-After header, fall back to policy delay
                            parse_retry_after_header(&headers)
                                .unwrap_or_else(|| self.default_policy.delay_for_attempt(attempt))
                        } else {
                            // Use policy delay for other retryable errors
                            self.default_policy.delay_for_attempt(attempt)
                        };

                        if delay > Duration::ZERO {
                            sleep(delay).await;
                        }
                    } else {
                        // Success! Return the response
                        return Ok(response);
                    }
                }
                Err(e) => {
                    last_error = Some(e);

                    // Check if error is retryable
                    if !classifier(last_error.as_ref().unwrap()) {
                        // Not retryable, return immediately
                        return Err(anyhow::anyhow!(last_error.unwrap())
                            .context("Non-retryable HTTP error"));
                    }

                    // Error is retryable, but check if we've exhausted retries
                    if attempt >= self.default_policy.max_attempts {
                        // No more retries, return the last error
                        break;
                    }

                    // Calculate delay for next retry
                    let delay = self.default_policy.delay_for_attempt(attempt);
                    if delay > Duration::ZERO {
                        log::debug!("Waiting {:?} before retry attempt {}", delay, attempt + 1);
                        sleep(delay).await;
                    }
                }
            }
        }

        // All retries exhausted, return the last error
        match last_error {
            Some(e) => Err(anyhow::anyhow!(e).context("HTTP request failed after retries")),
            None => Err(anyhow::anyhow!(
                "HTTP request failed but no error was recorded"
            )),
        }
    }

    /// Execute a request builder with custom retry policy
    ///
    /// Allows per-request policy override. The policy is cloned for this request.
    ///
    /// # Arguments
    ///
    /// * `request_builder` - A closure that returns a `RequestBuilder`
    /// * `policy` - Custom retry policy for this request
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use tui_framework::http_retry::RetryableHttpClient;
    /// # use reqwest::Client;
    /// # use tui_framework::retry::RetryPolicy;
    /// # use std::time::Duration;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let retry_client = RetryableHttpClient::new(Client::new());
    /// let custom_policy = RetryPolicy::fixed_delay(5, Duration::from_millis(100));
    /// let response = retry_client.execute_with_policy(
    ///     || retry_client.client().get("https://api.example.com/data"),
    ///     custom_policy
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn execute_with_policy<F>(
        &self,
        request_builder: F,
        policy: RetryPolicy,
    ) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
    {
        self.execute_with_policy_and_classifier(request_builder, policy, is_retryable_http_error)
            .await
    }

    /// Execute a request builder with custom retry policy and error classifier
    ///
    /// Internal method that handles both policy and classifier customization.
    async fn execute_with_policy_and_classifier<F, C>(
        &self,
        request_builder: F,
        policy: RetryPolicy,
        classifier: C,
    ) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
        C: Fn(&ReqwestError) -> bool + Send + Sync,
    {
        use tokio::time::sleep;

        let mut last_error: Option<ReqwestError> = None;

        log::debug!(
            "Starting HTTP request with retry (max attempts: {}, custom policy and classifier)",
            policy.max_attempts + 1
        );

        // Try up to max_attempts + 1 times (initial attempt + retries)
        for attempt in 0..=policy.max_attempts {
            // Build and execute the request
            let builder = request_builder();

            if attempt > 0 {
                log::debug!(
                    "Retry attempt {} of {} (custom policy and classifier)",
                    attempt,
                    policy.max_attempts
                );
            }

            let result = builder.send().await;

            match result {
                Ok(response) => {
                    // Check if response status indicates an error
                    let status = response.status();

                    // 4xx errors (except 429) should NOT be retried - return immediately
                    if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        // Convert to error and return immediately (not retryable)
                        let error = response.error_for_status().unwrap_err();
                        return Err(anyhow::anyhow!(error).context("Non-retryable HTTP error"));
                    }

                    // 5xx and 429 are retryable
                    if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        // Extract headers before consuming response
                        let headers = response.headers().clone();

                        // This is a retryable error status, convert to error
                        let error = response.error_for_status().unwrap_err();
                        last_error = Some(error);

                        // Check if error is retryable (should be, but verify)
                        if !classifier(last_error.as_ref().unwrap()) {
                            // Not retryable, return immediately
                            return Err(anyhow::anyhow!(last_error.unwrap())
                                .context("Non-retryable HTTP error"));
                        }

                        // Error is retryable, but check if we've exhausted retries
                        if attempt >= policy.max_attempts {
                            // No more retries, return the last error
                            break;
                        }

                        // Check for Retry-After header in 429 responses
                        let delay = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            // Try to parse Retry-After header, fall back to policy delay
                            parse_retry_after_header(&headers)
                                .unwrap_or_else(|| policy.delay_for_attempt(attempt))
                        } else {
                            // Use policy delay for other retryable errors
                            policy.delay_for_attempt(attempt)
                        };

                        if delay > Duration::ZERO {
                            sleep(delay).await;
                        }
                    } else {
                        // Success! Return the response
                        return Ok(response);
                    }
                }
                Err(e) => {
                    last_error = Some(e);

                    // Check if error is retryable
                    if !classifier(last_error.as_ref().unwrap()) {
                        // Not retryable, return immediately
                        return Err(anyhow::anyhow!(last_error.unwrap())
                            .context("Non-retryable HTTP error"));
                    }

                    // Error is retryable, but check if we've exhausted retries
                    if attempt >= policy.max_attempts {
                        // No more retries, return the last error
                        break;
                    }

                    // Calculate delay for next retry using the custom policy
                    let delay = policy.delay_for_attempt(attempt);
                    if delay > Duration::ZERO {
                        log::debug!(
                            "Waiting {:?} before retry attempt {} (network error, custom policy)",
                            delay,
                            attempt + 1
                        );
                        sleep(delay).await;
                    }
                }
            }
        }

        // All retries exhausted, return the last error
        match last_error {
            Some(e) => Err(anyhow::anyhow!(e).context("HTTP request failed after retries")),
            None => Err(anyhow::anyhow!(
                "HTTP request failed but no error was recorded"
            )),
        }
    }

    /// Execute a GET request with retry
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use tui_framework::http_retry::RetryableHttpClient;
    /// # use reqwest::Client;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let retry_client = RetryableHttpClient::new(Client::new());
    /// let response = retry_client.get("https://api.example.com/data").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.get(url)).await
    }

    /// Execute a POST request with retry
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use tui_framework::http_retry::RetryableHttpClient;
    /// # use reqwest::Client;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let retry_client = RetryableHttpClient::new(Client::new());
    /// let response = retry_client.post("https://api.example.com/data").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn post(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.post(url)).await
    }

    /// Execute a PUT request with retry
    pub async fn put(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.put(url)).await
    }

    /// Execute a DELETE request with retry
    pub async fn delete(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.delete(url)).await
    }

    /// Execute a PATCH request with retry
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use tui_framework::http_retry::RetryableHttpClient;
    /// # use reqwest::Client;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let retry_client = RetryableHttpClient::new(Client::new());
    /// let response = retry_client.patch("https://api.example.com/data").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn patch(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.patch(url)).await
    }

    /// Execute a HEAD request with retry
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use tui_framework::http_retry::RetryableHttpClient;
    /// # use reqwest::Client;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let retry_client = RetryableHttpClient::new(Client::new());
    /// let response = retry_client.head("https://api.example.com/data").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn head(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.head(url)).await
    }

    /// Execute an OPTIONS request with retry
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use tui_framework::http_retry::RetryableHttpClient;
    /// # use reqwest::Client;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let retry_client = RetryableHttpClient::new(Client::new());
    /// let response = retry_client.options("https://api.example.com/data").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn options(&self, url: &str) -> Result<Response> {
        use reqwest::Method;
        self.execute_with_retry(|| self.client.request(Method::OPTIONS, url))
            .await
    }
}

/// Parse Retry-After header from HTTP response headers
///
/// Supports both formats:
/// - Integer seconds (e.g., "60")
/// - HTTP-date (e.g., "Wed, 21 Oct 2015 07:28:00 GMT")
///
/// Returns `Some(Duration)` if header is present and valid, `None` otherwise.
pub(crate) fn parse_retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    use reqwest::header::RETRY_AFTER;

    let retry_after_value = headers.get(RETRY_AFTER)?;
    let retry_after_str = retry_after_value.to_str().ok()?.trim();

    // Try parsing as integer seconds first
    if let Ok(seconds) = retry_after_str.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    // Try parsing as HTTP-date
    if let Ok(http_date) = httpdate::parse_http_date(retry_after_str) {
        let now = std::time::SystemTime::now();
        // Convert HttpDate to SystemTime for duration calculation
        let future_time = std::time::SystemTime::from(http_date);
        if let Ok(duration) = future_time.duration_since(now) {
            return Some(duration);
        }
        // If date is in the past, return None (fallback to policy)
        return None;
    }

    // Invalid format, return None
    None
}
