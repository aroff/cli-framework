use crate::retry::policy::RetryPolicy;
use anyhow::Result;
use std::time::Duration;

/// Executor that applies retry policies to operations
pub struct RetryExecutor {
    policy: RetryPolicy,
}

impl RetryExecutor {
    /// Create a new retry executor with a policy
    pub fn new(policy: RetryPolicy) -> Self {
        Self { policy }
    }

    /// Execute an operation with retry logic
    ///
    /// The operation is retried according to the policy if it returns an error.
    /// Returns the first successful result or the last error if all retries fail.
    pub fn execute<F, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Result<T>,
    {
        let mut last_error = None;

        for attempt in 0..=self.policy.max_attempts {
            let result = operation();

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
                        std::thread::sleep(delay);
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

    // Note: Async support will be added in v2 when async runtime is available
}

impl Default for RetryExecutor {
    fn default() -> Self {
        Self::new(RetryPolicy::default())
    }
}
