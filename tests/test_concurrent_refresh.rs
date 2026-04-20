//! Unit tests for concurrent DataSource refresh operations
//!
//! Verifies that multiple DataSource refresh operations can execute concurrently.

use anyhow::Result;
use async_trait::async_trait;
use cli_framework::app::AppContext;
use cli_framework::data_source::DataSource;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration, Instant};

/// Test context
struct TestContext;

impl AppContext for TestContext {}

/// Test DataSource with configurable delay
struct DelayedDataSource {
    delay_ms: u64,
    refresh_count: Arc<AtomicUsize>,
    data: String,
}

#[async_trait]
impl DataSource for DelayedDataSource {
    type Row = String;

    fn len(&self) -> usize {
        1
    }

    fn get(&self, _index: usize) -> Option<&Self::Row> {
        Some(&self.data)
    }

    async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        sleep(Duration::from_millis(self.delay_ms)).await;
        self.refresh_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[tokio::test]
async fn test_concurrent_data_source_refresh() {
    // Test that multiple DataSource refresh operations can run concurrently
    let count1 = Arc::new(AtomicUsize::new(0));
    let count2 = Arc::new(AtomicUsize::new(0));
    let count3 = Arc::new(AtomicUsize::new(0));

    let mut ds1 = DelayedDataSource {
        delay_ms: 50,
        refresh_count: count1.clone(),
        data: "test1".to_string(),
    };

    let mut ds2 = DelayedDataSource {
        delay_ms: 50,
        refresh_count: count2.clone(),
        data: "test2".to_string(),
    };

    let mut ds3 = DelayedDataSource {
        delay_ms: 50,
        refresh_count: count3.clone(),
        data: "test3".to_string(),
    };

    let ctx = TestContext;

    // Start all refreshes concurrently
    let start = Instant::now();
    let (r1, r2, r3) = tokio::join!(ds1.refresh(&ctx), ds2.refresh(&ctx), ds3.refresh(&ctx),);

    let elapsed = start.elapsed();

    // Verify all completed successfully
    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert!(r3.is_ok());

    // Verify all were called
    assert_eq!(count1.load(Ordering::Relaxed), 1);
    assert_eq!(count2.load(Ordering::Relaxed), 1);
    assert_eq!(count3.load(Ordering::Relaxed), 1);

    // Verify they ran concurrently (should take ~50ms, not 150ms)
    assert!(elapsed >= Duration::from_millis(40));
    assert!(elapsed <= Duration::from_millis(100)); // Much less than 150ms if sequential
}

#[tokio::test]
async fn test_concurrent_refresh_with_different_delays() {
    // Test concurrent refreshes with different delays
    let count1 = Arc::new(AtomicUsize::new(0));
    let count2 = Arc::new(AtomicUsize::new(0));

    let mut ds1 = DelayedDataSource {
        delay_ms: 100,
        refresh_count: count1.clone(),
        data: "test1".to_string(),
    };

    let mut ds2 = DelayedDataSource {
        delay_ms: 50,
        refresh_count: count2.clone(),
        data: "test2".to_string(),
    };

    let ctx = TestContext;

    let start = Instant::now();
    let (r1, r2) = tokio::join!(ds1.refresh(&ctx), ds2.refresh(&ctx),);
    let elapsed = start.elapsed();

    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert_eq!(count1.load(Ordering::Relaxed), 1);
    assert_eq!(count2.load(Ordering::Relaxed), 1);

    // Should complete in ~100ms (max of the two delays), not 150ms
    assert!(elapsed >= Duration::from_millis(90));
    assert!(elapsed <= Duration::from_millis(150));
}
