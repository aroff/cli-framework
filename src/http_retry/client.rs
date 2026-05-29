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
/// use cli_framework::http_retry::RetryableHttpClient;
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
    /// use cli_framework::http_retry::RetryableHttpClient;
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
    /// use cli_framework::http_retry::RetryableHttpClient;
    /// use cli_framework::retry::RetryPolicy;
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
}

/// Returns a reqwest::Client with secure defaults: 5s connect timeout, 30s total
/// timeout, built-in TLS roots, TLS certificate verification enabled.
///
/// This factory MUST NOT call `danger_accept_invalid_certs(true)`.
pub fn secure_reqwest_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(90))
        .tls_built_in_root_certs(true)
        .build()?)
}

impl RetryableHttpClient {
    async fn retry_loop<F, C>(
        &self,
        policy: &RetryPolicy,
        request_builder: F,
        classifier: C,
    ) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
        C: Fn(&ReqwestError) -> bool + Send + Sync,
    {
        use tokio::time::sleep;

        let mut last_error: Option<ReqwestError> = None;

        for attempt in 0..=policy.max_attempts {
            let builder = request_builder();

            match builder.send().await {
                Ok(response) => {
                    let status = response.status();

                    if status.is_client_error() || status.is_server_error() {
                        let headers = response.headers().clone();
                        let error = response.error_for_status().unwrap_err();

                        if !classifier(&error) {
                            return Err(anyhow::anyhow!(error).context("Non-retryable HTTP error"));
                        }

                        last_error = Some(error);

                        if attempt >= policy.max_attempts {
                            break;
                        }

                        let delay = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            parse_retry_after_header(&headers)
                                .unwrap_or_else(|| policy.delay_for_attempt(attempt))
                        } else {
                            policy.delay_for_attempt(attempt)
                        };

                        if delay > Duration::ZERO {
                            sleep(delay).await;
                        }
                    } else {
                        return Ok(response);
                    }
                }
                Err(e) => {
                    if !classifier(&e) {
                        return Err(anyhow::anyhow!(e).context("Non-retryable HTTP error"));
                    }

                    last_error = Some(e);

                    if attempt >= policy.max_attempts {
                        break;
                    }

                    let delay = policy.delay_for_attempt(attempt);
                    if delay > Duration::ZERO {
                        sleep(delay).await;
                    }
                }
            }
        }

        match last_error {
            Some(e) => Err(anyhow::anyhow!(e).context("HTTP request failed after retries")),
            None => Err(anyhow::anyhow!(
                "HTTP request failed but no error was recorded"
            )),
        }
    }

    pub async fn execute_with_retry<F>(&self, request_builder: F) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
    {
        self.retry_loop(
            &self.default_policy,
            request_builder,
            is_retryable_http_error,
        )
        .await
    }

    pub async fn execute_with_classifier<F, C>(
        &self,
        request_builder: F,
        classifier: C,
    ) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
        C: Fn(&ReqwestError) -> bool + Send + Sync,
    {
        self.retry_loop(&self.default_policy, request_builder, classifier)
            .await
    }

    pub async fn execute_with_policy<F>(
        &self,
        request_builder: F,
        policy: RetryPolicy,
    ) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
    {
        self.retry_loop(&policy, request_builder, is_retryable_http_error)
            .await
    }

    pub async fn execute_with_policy_and_classifier<F, C>(
        &self,
        request_builder: F,
        policy: RetryPolicy,
        classifier: C,
    ) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
        C: Fn(&ReqwestError) -> bool + Send + Sync,
    {
        self.retry_loop(&policy, request_builder, classifier).await
    }

    /// Execute a GET request with retry
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use cli_framework::http_retry::RetryableHttpClient;
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
    /// # use cli_framework::http_retry::RetryableHttpClient;
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
    /// # use cli_framework::http_retry::RetryableHttpClient;
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
    /// # use cli_framework::http_retry::RetryableHttpClient;
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
    /// # use cli_framework::http_retry::RetryableHttpClient;
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
        let future_time = http_date;
        if let Ok(duration) = future_time.duration_since(now) {
            return Some(duration);
        }
        // If date is in the past, return None (fallback to policy)
        return None;
    }

    // Invalid format, return None
    None
}
