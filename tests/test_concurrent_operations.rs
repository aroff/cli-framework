//! Unit tests for concurrent async operations
//!
//! Verifies that multiple async operations can run concurrently without blocking each other.

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
struct TimedDataSource {
    delay_ms: u64,
    execution_count: Arc<AtomicUsize>,
    data: String,
}

#[async_trait]
impl DataSource for TimedDataSource {
    type Row = String;

    fn len(&self) -> usize {
        1
    }

    fn get(&self, _index: usize) -> Option<&Self::Row> {
        Some(&self.data)
    }

    async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        sleep(Duration::from_millis(self.delay_ms)).await;
        self.execution_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[tokio::test]
async fn test_concurrent_operations_dont_block_each_other() {
    // Test that multiple async operations can run concurrently
    let count1 = Arc::new(AtomicUsize::new(0));
    let count2 = Arc::new(AtomicUsize::new(0));
    let count3 = Arc::new(AtomicUsize::new(0));

    let mut op1 = TimedDataSource {
        delay_ms: 100,
        execution_count: count1.clone(),
        data: "op1".to_string(),
    };

    let mut op2 = TimedDataSource {
        delay_ms: 100,
        execution_count: count2.clone(),
        data: "op2".to_string(),
    };

    let mut op3 = TimedDataSource {
        delay_ms: 100,
        execution_count: count3.clone(),
        data: "op3".to_string(),
    };

    let ctx = TestContext;

    // Start all operations concurrently
    let start = Instant::now();
    let (r1, r2, r3) = tokio::join!(op1.refresh(&ctx), op2.refresh(&ctx), op3.refresh(&ctx),);

    let elapsed = start.elapsed();

    // Verify all completed
    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert!(r3.is_ok());

    // Verify all executed
    assert_eq!(count1.load(Ordering::Relaxed), 1);
    assert_eq!(count2.load(Ordering::Relaxed), 1);
    assert_eq!(count3.load(Ordering::Relaxed), 1);

    // Verify they ran concurrently (should take ~100ms, not 300ms)
    assert!(elapsed >= Duration::from_millis(90));
    assert!(elapsed <= Duration::from_millis(150)); // Much less than 300ms if sequential
}

#[tokio::test]
async fn test_operations_can_be_interrupted_by_user_input() {
    // Test that user input can be processed even during operations
    let operation_count = Arc::new(AtomicUsize::new(0));
    let user_input_count = Arc::new(AtomicUsize::new(0));

    let mut data_source = TimedDataSource {
        delay_ms: 500,
        execution_count: operation_count.clone(),
        data: "test".to_string(),
    };

    let ctx = TestContext;

    // Start operation
    let operation_handle = tokio::spawn(async move { data_source.refresh(&ctx).await });

    // Simulate user input during operation
    for _ in 0..5 {
        sleep(Duration::from_millis(50)).await;
        user_input_count.fetch_add(1, Ordering::Relaxed);
    }

    // Wait for operation
    operation_handle.await.unwrap().unwrap();

    // Verify both happened
    assert_eq!(operation_count.load(Ordering::Relaxed), 1);
    assert_eq!(user_input_count.load(Ordering::Relaxed), 5);
}
