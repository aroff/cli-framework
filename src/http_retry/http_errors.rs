//! HTTP error classification utilities
//!
//! Provides functions to determine whether HTTP errors should trigger retry attempts.

use reqwest::Error;

/// Check if an HTTP error should be retried
///
/// Returns `true` for:
/// - Network errors (connection failures, timeouts)
/// - 5xx server errors
/// - 429 Too Many Requests (rate limiting)
///
/// Returns `false` for:
/// - 4xx client errors (except 429)
/// - Request construction errors
///
/// # Example
///
/// ```rust,no_run
/// use cli_framework::http_retry::http_errors::is_retryable_http_error;
/// use reqwest::Error;
///
/// # async fn example() -> anyhow::Result<()> {
/// # let error: Error = todo!();
/// if is_retryable_http_error(&error) {
///     // Retry the request
/// }
/// # Ok(())
/// # }
/// ```
pub fn is_retryable_http_error(error: &Error) -> bool {
    if let Some(status) = error.status() {
        // Retry on server errors and rate limiting
        status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    } else {
        // Network errors are generally retryable
        error.is_timeout() || error.is_connect() || error.is_request()
    }
}

/// Check if error is a timeout
///
/// # Example
///
/// ```rust,no_run
/// use cli_framework::http_retry::http_errors::is_timeout;
/// use reqwest::Error;
///
/// # async fn example() -> anyhow::Result<()> {
/// # let error: Error = todo!();
/// if is_timeout(&error) {
///     // Handle timeout
/// }
/// # Ok(())
/// # }
/// ```
pub fn is_timeout(error: &Error) -> bool {
    error.is_timeout()
}

/// Check if error is a connection error
///
/// # Example
///
/// ```rust,no_run
/// use cli_framework::http_retry::http_errors::is_connection_error;
/// use reqwest::Error;
///
/// # async fn example() -> anyhow::Result<()> {
/// # let error: Error = todo!();
/// if is_connection_error(&error) {
///     // Handle connection error
/// }
/// # Ok(())
/// # }
/// ```
pub fn is_connection_error(error: &Error) -> bool {
    error.is_connect()
}
