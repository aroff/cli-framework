//! Progress indicator utilities
//!
//! Provides progress bars for long-running operations with rate limiting
//! and graceful degradation.
//!
//! This module requires the "progress" feature to be enabled.

#[cfg(feature = "progress")]
use indicatif::{ProgressBar, ProgressStyle};
#[cfg(feature = "progress")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "progress")]
use std::time::{Duration, Instant};

#[cfg(feature = "progress")]
/// Rate-limited progress bar wrapper
struct RateLimitedProgressBar {
    inner: ProgressBar,
    last_update: Arc<Mutex<Instant>>,
    min_interval: Duration,
}

#[cfg(feature = "progress")]
impl RateLimitedProgressBar {
    fn new(pb: ProgressBar) -> Self {
        Self {
            inner: pb,
            last_update: Arc::new(Mutex::new(Instant::now())),
            min_interval: Duration::from_millis(100), // 10 updates/second (FR-008a)
        }
    }

    fn set_position(&self, pos: u64) {
        let mut last = self.last_update.lock().unwrap();
        let now = Instant::now();

        if now.duration_since(*last) >= self.min_interval {
            self.inner.set_position(pos);
            *last = now;
        }
    }

    fn inc(&self, delta: u64) {
        let mut last = self.last_update.lock().unwrap();
        let now = Instant::now();

        if now.duration_since(*last) >= self.min_interval {
            self.inner.inc(delta);
            *last = now;
        }
    }

    fn set_message(&self, msg: impl Into<String>) {
        self.inner.set_message(msg.into());
    }

    fn finish(&self) {
        self.inner.finish();
    }
}

/// Create a progress bar for long-running operations
///
/// # Arguments
///
/// * `total` - Total value for determinate progress. Use 0 for indeterminate progress.
///
/// # Returns
///
/// Progress bar instance (FR-008)
///
/// # Example
///
/// ```rust,ignore
/// #[cfg(feature = "progress")]
/// use cli_framework::cli_output::progress::create_progress_bar;
///
/// let pb = create_progress_bar(100);
/// pb.set_message("Processing...");
/// for i in 0..100 {
///     pb.set_position(i);
///     // ... do work ...
/// }
/// pb.finish();
/// ```
#[cfg(feature = "progress")]
pub fn create_progress_bar(total: u64) -> impl ProgressBarTrait {
    let pb = if total == 0 {
        // Indeterminate progress (FR-008)
        ProgressBar::new_spinner()
    } else {
        // Determinate progress (FR-008)
        ProgressBar::new(total)
    };

    // Configure style for graceful degradation (FR-007)
    let style = ProgressStyle::default_bar()
        .template("{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})")
        .unwrap_or_else(|_| ProgressStyle::default_spinner());

    pb.set_style(style);

    // Wrap in rate limiter (FR-008a)
    RateLimitedProgressBar::new(pb)
}

#[cfg(feature = "progress")]
/// Trait for progress bar operations
pub trait ProgressBarTrait {
    fn set_position(&self, pos: u64);
    fn inc(&self, delta: u64);
    fn set_message(&self, msg: impl Into<String>);
    fn finish(&self);
    fn length(&self) -> Option<u64>;
    fn position(&self) -> u64;
}

#[cfg(feature = "progress")]
impl ProgressBarTrait for RateLimitedProgressBar {
    fn set_position(&self, pos: u64) {
        RateLimitedProgressBar::set_position(self, pos);
    }

    fn inc(&self, delta: u64) {
        RateLimitedProgressBar::inc(self, delta);
    }

    fn set_message(&self, msg: impl Into<String>) {
        RateLimitedProgressBar::set_message(self, msg);
    }

    fn finish(&self) {
        RateLimitedProgressBar::finish(self);
    }

    fn length(&self) -> Option<u64> {
        self.inner.length()
    }

    fn position(&self) -> u64 {
        self.inner.position()
    }
}

// Stub implementation when feature is not enabled
#[cfg(not(feature = "progress"))]
pub fn create_progress_bar(_total: u64) {
    // No-op when feature is disabled
    // Applications should check for feature availability
}
