//! Unit tests for the `retry` module — `RetryPolicy`, `RetryExecutor`,
//! and `AsyncRetryExecutor`.
//!
//! These exercise behavior through the crate's public API only
//! (`cli_framework::retry::*`): how many times an operation is invoked,
//! what is returned on success/exhaustion, how the backoff delay is
//! computed for each strategy, and how the async classifier and per-attempt
//! timeout influence retrying. They are written to survive internal
//! refactors — nothing here reaches into private fields beyond the
//! documented public surface.

use cli_framework::retry::policy::RetryStrategy;
use cli_framework::retry::{AsyncRetryExecutor, RetryExecutor, RetryPolicy};
use std::cell::Cell;
use std::time::Duration;

#[test]
fn sync_executor_returns_value_and_calls_once_on_first_success() {
    let calls = Cell::new(0u32);
    let executor = RetryExecutor::new(RetryPolicy::no_retries());

    let result: anyhow::Result<&str> = executor.execute(|| {
        calls.set(calls.get() + 1);
        Ok("ok")
    });

    assert_eq!(result.unwrap(), "ok");
    assert_eq!(
        calls.get(),
        1,
        "a succeeding operation must run exactly once"
    );
}

#[test]
fn sync_executor_retries_until_success_then_returns_value() {
    let calls = Cell::new(0u32);
    // Allow up to 5 retries; no inter-attempt delay so the test is instant.
    let executor = RetryExecutor::new(RetryPolicy::new(5, RetryStrategy::None));

    let result: anyhow::Result<u32> = executor.execute(|| {
        calls.set(calls.get() + 1);
        if calls.get() < 3 {
            Err(anyhow::anyhow!("transient failure"))
        } else {
            Ok(calls.get())
        }
    });

    assert_eq!(result.unwrap(), 3);
    assert_eq!(
        calls.get(),
        3,
        "should stop calling once the operation succeeds"
    );
}

#[test]
fn sync_executor_exhausts_all_attempts_then_returns_last_error() {
    let calls = Cell::new(0u32);
    // max_attempts = 2 means 1 initial try + 2 retries = 3 total invocations.
    let executor = RetryExecutor::new(RetryPolicy::new(2, RetryStrategy::None));

    let result: anyhow::Result<()> = executor.execute(|| {
        calls.set(calls.get() + 1);
        Err(anyhow::anyhow!("attempt {}", calls.get()))
    });

    let err = result.unwrap_err();
    assert_eq!(
        calls.get(),
        3,
        "max_attempts=2 must produce exactly 3 invocations (1 initial + 2 retries)"
    );
    assert_eq!(
        err.to_string(),
        "attempt 3",
        "the error returned must be the one from the final attempt"
    );
}

#[test]
fn sync_executor_no_retries_policy_calls_once_on_failure() {
    let calls = Cell::new(0u32);
    let executor = RetryExecutor::new(RetryPolicy::no_retries());

    let result: anyhow::Result<()> = executor.execute(|| {
        calls.set(calls.get() + 1);
        Err(anyhow::anyhow!("boom"))
    });

    assert!(result.is_err());
    assert_eq!(
        calls.get(),
        1,
        "a no_retries policy must invoke the operation exactly once even on failure"
    );
}

// --- delay_for_attempt: backoff math ---------------------------------------

#[test]
fn delay_none_strategy_is_always_zero() {
    let policy = RetryPolicy::new(5, RetryStrategy::None);
    assert_eq!(policy.delay_for_attempt(0), Duration::ZERO);
    assert_eq!(policy.delay_for_attempt(3), Duration::ZERO);
}

#[test]
fn delay_fixed_strategy_returns_constant_delay() {
    let policy = RetryPolicy::fixed_delay(5, Duration::from_millis(250));
    assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(250));
    assert_eq!(policy.delay_for_attempt(4), Duration::from_millis(250));
}

#[test]
fn delay_is_zero_once_attempt_reaches_max_attempts() {
    // The delay is only meaningful for attempts that will be retried; at or
    // beyond max_attempts there is no further retry, so the delay collapses
    // to zero regardless of the configured strategy.
    let policy = RetryPolicy::fixed_delay(3, Duration::from_millis(500));
    assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(500));
    assert_eq!(policy.delay_for_attempt(3), Duration::ZERO);
    assert_eq!(policy.delay_for_attempt(99), Duration::ZERO);
}

#[test]
fn delay_exponential_doubles_each_attempt() {
    let policy =
        RetryPolicy::exponential_backoff(10, Duration::from_millis(100), Duration::from_secs(60));
    assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
    assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(200));
    assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(400));
    assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(800));
}

#[test]
fn delay_exponential_is_capped_at_max() {
    let policy = RetryPolicy::exponential_backoff(
        10,
        Duration::from_millis(100),
        Duration::from_millis(250),
    );
    // attempt 2 would be 400ms unclamped, but max caps it at 250ms.
    assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(250));
}

#[test]
fn delay_exponential_shift_is_clamped_at_attempt_10() {
    // The implementation clamps the shift exponent at 10 to avoid overflow,
    // so attempts 10 and 11 yield the same (uncapped) delay.
    let policy =
        RetryPolicy::exponential_backoff(20, Duration::from_millis(1), Duration::from_secs(3600));
    let at_10 = policy.delay_for_attempt(10);
    let at_11 = policy.delay_for_attempt(11);
    assert_eq!(at_10, Duration::from_millis(1024));
    assert_eq!(
        at_11, at_10,
        "shift exponent must be clamped so very high attempts do not overflow"
    );
}

#[test]
fn delay_linear_grows_by_increment_each_attempt() {
    let policy = RetryPolicy::new(
        10,
        RetryStrategy::Linear {
            initial: Duration::from_millis(100),
            increment: Duration::from_millis(50),
            max: Duration::from_secs(60),
        },
    );
    assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
    assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(150));
    assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(200));
}

#[test]
fn delay_linear_is_capped_at_max() {
    let policy = RetryPolicy::new(
        10,
        RetryStrategy::Linear {
            initial: Duration::from_millis(100),
            increment: Duration::from_millis(100),
            max: Duration::from_millis(250),
        },
    );
    // attempt 3 would be 100 + 300 = 400ms unclamped, capped to 250ms.
    assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(250));
}

#[test]
fn default_policy_retries_three_times_with_fixed_half_second_delay() {
    // The default policy is the documented "3 retries, 500ms fixed" contract.
    // Verified through the pure delay path so the test stays fast.
    let policy = RetryPolicy::default();
    assert_eq!(policy.max_attempts, 3);
    assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(500));
    assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(500));
}

// --- AsyncRetryExecutor ----------------------------------------------------

#[tokio::test]
async fn async_executor_returns_value_on_first_success() {
    let calls = Cell::new(0u32);
    let executor = AsyncRetryExecutor::new(RetryPolicy::no_retries());

    let result: anyhow::Result<&str> = executor
        .execute(|| async {
            calls.set(calls.get() + 1);
            Ok("done")
        })
        .await;

    assert_eq!(result.unwrap(), "done");
    assert_eq!(calls.get(), 1);
}

#[tokio::test]
async fn async_executor_retries_until_success() {
    let calls = Cell::new(0u32);
    let executor = AsyncRetryExecutor::new(RetryPolicy::new(5, RetryStrategy::None));

    let result: anyhow::Result<u32> = executor
        .execute(|| async {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                Err(anyhow::anyhow!("transient"))
            } else {
                Ok(calls.get())
            }
        })
        .await;

    assert_eq!(result.unwrap(), 3);
    assert_eq!(calls.get(), 3);
}

#[tokio::test]
async fn async_executor_exhausts_attempts_then_returns_last_error() {
    let calls = Cell::new(0u32);
    let executor = AsyncRetryExecutor::new(RetryPolicy::new(2, RetryStrategy::None));

    let result: anyhow::Result<()> = executor
        .execute(|| async {
            calls.set(calls.get() + 1);
            Err(anyhow::anyhow!("attempt {}", calls.get()))
        })
        .await;

    assert_eq!(calls.get(), 3, "1 initial + 2 retries");
    assert_eq!(result.unwrap_err().to_string(), "attempt 3");
}

#[tokio::test]
async fn async_classifier_stops_retrying_on_non_retryable_error() {
    let calls = Cell::new(0u32);
    // Classifier marks anything containing "fatal" as non-retryable.
    let executor = AsyncRetryExecutor::new(RetryPolicy::new(5, RetryStrategy::None))
        .with_classifier(|e| !e.to_string().contains("fatal"));

    let result: anyhow::Result<()> = executor
        .execute(|| async {
            calls.set(calls.get() + 1);
            Err(anyhow::anyhow!("fatal: do not retry"))
        })
        .await;

    assert!(result.is_err());
    assert_eq!(
        calls.get(),
        1,
        "a non-retryable error must stop the loop immediately"
    );
}

#[tokio::test]
async fn async_classifier_retries_retryable_errors_until_exhaustion() {
    let calls = Cell::new(0u32);
    let executor = AsyncRetryExecutor::new(RetryPolicy::new(3, RetryStrategy::None))
        .with_classifier(|e| e.to_string().contains("retry-me"));

    let result: anyhow::Result<()> = executor
        .execute(|| async {
            calls.set(calls.get() + 1);
            Err(anyhow::anyhow!("retry-me please"))
        })
        .await;

    assert!(result.is_err());
    assert_eq!(
        calls.get(),
        4,
        "retryable error retried to exhaustion: 1 + 3"
    );
}

// --- per-attempt timeout ---------------------------------------------------

#[tokio::test]
async fn async_completes_within_timeout_without_error() {
    let executor = AsyncRetryExecutor::new(
        RetryPolicy::new(2, RetryStrategy::None).with_timeout(Duration::from_secs(5)),
    );

    let result: anyhow::Result<&str> = executor.execute(|| async { Ok("fast") }).await;

    assert_eq!(result.unwrap(), "fast");
}

#[tokio::test]
async fn async_slow_attempt_times_out_and_retries_by_default() {
    let calls = Cell::new(0u32);
    // Default retry_on_timeout is true, so a timed-out attempt is retried.
    let executor = AsyncRetryExecutor::new(
        RetryPolicy::new(2, RetryStrategy::None).with_timeout(Duration::from_millis(20)),
    );

    let result: anyhow::Result<()> = executor
        .execute(|| async {
            calls.set(calls.get() + 1);
            // Outlives the per-attempt timeout; the executor cancels it.
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(())
        })
        .await;

    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("timed out"),
        "expected a timeout error, got: {err}"
    );
    assert_eq!(
        calls.get(),
        3,
        "with retry_on_timeout=true (default), each timed-out attempt is retried: 1 + 2"
    );
}

#[tokio::test]
async fn async_timeout_is_not_retried_when_retry_on_timeout_is_false() {
    let calls = Cell::new(0u32);
    let executor = AsyncRetryExecutor::new(
        RetryPolicy::new(3, RetryStrategy::None)
            .with_timeout(Duration::from_millis(20))
            .with_retry_on_timeout(false),
    );

    let result: anyhow::Result<()> = executor
        .execute(|| async {
            calls.set(calls.get() + 1);
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(())
        })
        .await;

    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("timed out"),
        "expected a timeout error, got: {err}"
    );
    assert_eq!(
        calls.get(),
        1,
        "with retry_on_timeout=false, a timed-out attempt must NOT be retried"
    );
}

#[tokio::test]
async fn async_non_timeout_error_still_retries_when_retry_on_timeout_is_false() {
    // retry_on_timeout=false must only suppress retries for *timeout* errors,
    // not for ordinary operation errors.
    let calls = Cell::new(0u32);
    let executor = AsyncRetryExecutor::new(
        RetryPolicy::new(2, RetryStrategy::None)
            .with_timeout(Duration::from_secs(5))
            .with_retry_on_timeout(false),
    );

    let result: anyhow::Result<()> = executor
        .execute(|| async {
            calls.set(calls.get() + 1);
            Err(anyhow::anyhow!("ordinary failure"))
        })
        .await;

    assert!(result.is_err());
    assert_eq!(
        calls.get(),
        3,
        "ordinary errors are still retried even when retry_on_timeout=false"
    );
}
