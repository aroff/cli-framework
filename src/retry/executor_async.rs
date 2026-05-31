use crate::retry::policy::RetryPolicy;
use anyhow::Result;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Async executor that applies retry policies to async operations.
/// If `classifier` is set, only errors for which it returns `true` are retried.
/// If `classifier` is `None`, all errors are retried (backward-compatible default).
type ErrorClassifier = Arc<dyn Fn(&anyhow::Error) -> bool + Send + Sync>;

pub struct AsyncRetryExecutor {
    policy: RetryPolicy,
    classifier: Option<ErrorClassifier>,
}

impl AsyncRetryExecutor {
    pub fn new(policy: RetryPolicy) -> Self {
        Self {
            policy,
            classifier: None,
        }
    }

    /// Attach a retryability classifier. Errors for which `f` returns `false` stop retrying.
    pub fn with_classifier<F>(mut self, f: F) -> Self
    where
        F: Fn(&anyhow::Error) -> bool + Send + Sync + 'static,
    {
        self.classifier = Some(Arc::new(f));
        self
    }

    /// Execute an async operation with retry logic and the configured classifier.
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut last_error = None;

        for attempt in 0..=self.policy.max_attempts {
            // Track whether this attempt failed specifically due to the
            // per-attempt timeout, so `retry_on_timeout` can be honored below.
            let (result, timed_out): (Result<T>, bool) =
                if let Some(timeout_duration) = self.policy.timeout {
                    match tokio::time::timeout(timeout_duration, operation()).await {
                        Ok(Ok(value)) => (Ok(value), false),
                        Ok(Err(e)) => (Err(e), false),
                        Err(_) => (Err(anyhow::anyhow!("Operation timed out")), true),
                    }
                } else {
                    (operation().await, false)
                };

            match result {
                Ok(value) => return Ok(value),
                Err(e) => {
                    // A timeout is retried only when the policy opts in.
                    if timed_out && !self.policy.retry_on_timeout {
                        return Err(e);
                    }
                    if let Some(ref clf) = self.classifier {
                        if !clf(&e) {
                            return Err(e);
                        }
                    }
                    last_error = Some(e);

                    if attempt >= self.policy.max_attempts {
                        break;
                    }

                    let delay = self.policy.delay_for_attempt(attempt);
                    if delay > Duration::ZERO {
                        sleep(delay).await;
                    }
                }
            }
        }

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
