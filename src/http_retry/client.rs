//! Retryable HTTP client implementation.

use crate::retry::executor_async::AsyncRetryExecutor;
use crate::retry::policy::RetryPolicy;
use anyhow::Result;
use reqwest::{Client, Error as ReqwestError, RequestBuilder, Response};
use std::time::Duration;

use super::http_errors::is_retryable_http_error;

/// HTTP client with automatic retry logic.
///
/// Wraps a `reqwest::Client` and provides automatic retry for transient failures
/// using the framework's `AsyncRetryExecutor` and `RetryPolicy`.
pub struct RetryableHttpClient {
    client: Client,
    default_policy: RetryPolicy,
}

impl RetryableHttpClient {
    pub fn new(client: Client) -> Self {
        let default_policy =
            RetryPolicy::exponential_backoff(3, Duration::from_secs(1), Duration::from_secs(10));
        Self {
            client,
            default_policy,
        }
    }

    pub fn with_policy(client: Client, policy: RetryPolicy) -> Self {
        Self {
            client,
            default_policy: policy,
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
}

/// Returns a `reqwest::Client` with secure defaults: 5s connect, 30s total timeout, TLS verification.
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
    pub async fn execute_with_retry<F>(&self, request_builder: F) -> Result<Response>
    where
        F: Fn() -> RequestBuilder + Send + Sync,
    {
        self.execute_with_policy_and_classifier(
            request_builder,
            self.default_policy.clone(),
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
        C: Fn(&ReqwestError) -> bool + Send + Sync + 'static,
    {
        self.execute_with_policy_and_classifier(
            request_builder,
            self.default_policy.clone(),
            classifier,
        )
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
        self.execute_with_policy_and_classifier(request_builder, policy, is_retryable_http_error)
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
        C: Fn(&ReqwestError) -> bool + Send + Sync + 'static,
    {
        let executor = AsyncRetryExecutor::new(policy).with_classifier(move |e: &anyhow::Error| {
            e.downcast_ref::<ReqwestError>()
                .map(&classifier)
                .unwrap_or(false)
        });

        executor
            .execute(|| async {
                let resp = request_builder()
                    .send()
                    .await
                    .map_err(anyhow::Error::from)?;

                let status = resp.status();
                if status.is_client_error() || status.is_server_error() {
                    let headers = resp.headers().clone();
                    let reqwest_err = resp.error_for_status().unwrap_err();
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        if let Some(delay) = parse_retry_after_header(&headers) {
                            tokio::time::sleep(delay).await;
                        }
                    }
                    Err(anyhow::Error::from(reqwest_err))
                } else {
                    Ok(resp)
                }
            })
            .await
    }

    pub async fn get(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.get(url)).await
    }

    pub async fn post(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.post(url)).await
    }

    pub async fn put(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.put(url)).await
    }

    pub async fn delete(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.delete(url)).await
    }

    pub async fn patch(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.patch(url)).await
    }

    pub async fn head(&self, url: &str) -> Result<Response> {
        self.execute_with_retry(|| self.client.head(url)).await
    }

    pub async fn options(&self, url: &str) -> Result<Response> {
        use reqwest::Method;
        self.execute_with_retry(|| self.client.request(Method::OPTIONS, url))
            .await
    }
}

/// Parse Retry-After header from HTTP response headers.
///
/// Supports integer seconds (e.g., "60") and HTTP-date format.
/// Returns `None` if the header is absent, unparseable, or in the past.
pub(crate) fn parse_retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    use reqwest::header::RETRY_AFTER;

    let retry_after_value = headers.get(RETRY_AFTER)?;
    let retry_after_str = retry_after_value.to_str().ok()?.trim();

    if let Ok(seconds) = retry_after_str.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    if let Ok(http_date) = httpdate::parse_http_date(retry_after_str) {
        let now = std::time::SystemTime::now();
        if let Ok(duration) = http_date.duration_since(now) {
            return Some(duration);
        }
        return None;
    }

    None
}
