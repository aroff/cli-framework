//! RetryPolicy configuration
//!
//! Defines retry policies for network operations with configurable strategies

use std::time::Duration;

/// Retry strategy for handling failures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryStrategy {
    /// No retries
    None,
    /// Fixed delay between retries
    Fixed(Duration),
    /// Exponential backoff (delay doubles each retry)
    Exponential {
        /// Initial delay
        initial: Duration,
        /// Maximum delay cap
        max: Duration,
    },
    /// Linear backoff (delay increases linearly)
    Linear {
        /// Initial delay
        initial: Duration,
        /// Increment per retry
        increment: Duration,
        /// Maximum delay cap
        max: Duration,
    },
}

/// Retry policy configuration
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (0 = no retries, 1 = one retry, etc.)
    pub max_attempts: u32,
    /// Strategy for delay between retries
    pub strategy: RetryStrategy,
    /// Timeout for each attempt
    pub timeout: Option<Duration>,
    /// Whether to retry on timeout
    pub retry_on_timeout: bool,
}

impl RetryPolicy {
    /// Create a new retry policy
    pub fn new(max_attempts: u32, strategy: RetryStrategy) -> Self {
        Self {
            max_attempts,
            strategy,
            timeout: None,
            retry_on_timeout: true,
        }
    }

    /// Create a policy with no retries
    pub fn no_retries() -> Self {
        Self {
            max_attempts: 0,
            strategy: RetryStrategy::None,
            timeout: None,
            retry_on_timeout: false,
        }
    }

    /// Create a policy with fixed delay retries
    pub fn fixed_delay(max_attempts: u32, delay: Duration) -> Self {
        Self {
            max_attempts,
            strategy: RetryStrategy::Fixed(delay),
            timeout: None,
            retry_on_timeout: true,
        }
    }

    /// Create a policy with exponential backoff
    pub fn exponential_backoff(
        max_attempts: u32,
        initial: Duration,
        max: Duration,
    ) -> Self {
        Self {
            max_attempts,
            strategy: RetryStrategy::Exponential { initial, max },
            timeout: None,
            retry_on_timeout: true,
        }
    }

    /// Set timeout for each attempt
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set whether to retry on timeout
    pub fn with_retry_on_timeout(mut self, retry: bool) -> Self {
        self.retry_on_timeout = retry;
        self
    }

    /// Calculate delay for a specific retry attempt (0-indexed)
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt >= self.max_attempts {
            return Duration::ZERO;
        }

        match self.strategy {
            RetryStrategy::None => Duration::ZERO,
            RetryStrategy::Fixed(delay) => delay,
            RetryStrategy::Exponential { initial, max } => {
                let delay = initial.as_millis() as u64 * (1 << attempt.min(10));
                Duration::from_millis(delay.min(max.as_millis() as u64))
            }
            RetryStrategy::Linear {
                initial,
                increment,
                max,
            } => {
                let delay = initial + increment * attempt;
                delay.min(max)
            }
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::fixed_delay(
            3,
            Duration::from_millis(500),
        )
    }
}
