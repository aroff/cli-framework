//! Async retry execution logic
//!
//! Executes async operations with retry policies

use crate::retry::policy::RetryPolicy;
use anyhow::Result;
use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;

/// Async executor that applies retry policies to async operations
pub struct AsyncRetryExecutor {
    policy: RetryPolicy,
}

impl AsyncRetryExecutor {
    /// Create a new async retry executor with a policy
    pub fn new(policy: RetryPolicy) -> Self {
        Self { policy }
    }

    /// Execute an async operation with retry logic
    ///
    /// The operation is retried according to the policy if it returns an error.
    /// Returns the first successful result or the last error if all retries fail.
    /// T081: Async-compatible retry executor
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut last_error = None;

        for attempt in 0..=self.policy.max_attempts {
            // Execute the operation
            let result = if let Some(timeout_duration) = self.policy.timeout {
                // T081: Implement timeout for async operations
                match tokio::time::timeout(timeout_duration, operation()).await {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(anyhow::anyhow!("Operation timed out")),
                }
            } else {
                operation().await
            };

            match result {
                Ok(value) => return Ok(value),
                Err(e) => {
                    last_error = Some(e);

                    // Don't retry if this was the last attempt
                    if attempt >= self.policy.max_attempts {
                        break;
                    }

                    // Calculate delay for next retry
                    let delay = self.policy.delay_for_attempt(attempt);
                    if delay > Duration::ZERO {
                        sleep(delay).await;
                    }
                }
            }
        }

        // Return the last error
        match last_error {
            Some(e) => Err(e),
            None => Err(anyhow::anyhow!(
                "Operation failed but no error was recorded"
            )),
        }
    }
}

impl Default for AsyncRetryExecutor {
    fn default() -> Self {
        Self::new(RetryPolicy::default())
    }
}
