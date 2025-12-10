//! Contract tests for async DataSource trait
//!
//! Verifies that DataSource implementations correctly implement the async refresh contract.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tui_framework::app::AppContext;
use tui_framework::data_source::DataSource;

/// Test context for async DataSource tests
struct TestContext {
    refresh_count: Arc<AtomicUsize>,
}

impl AppContext for TestContext {}

/// Test DataSource that simulates async network operations
struct AsyncTestDataSource {
    data: Vec<String>,
    refresh_count: Arc<AtomicUsize>,
}

#[async_trait]
impl DataSource for AsyncTestDataSource {
    type Row = String;

    fn len(&self) -> usize {
        self.data.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.data.get(index)
    }

    async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        // Simulate async network operation
        sleep(Duration::from_millis(10)).await;

        // Update data
        self.data = vec![
            "Item 1".to_string(),
            "Item 2".to_string(),
            "Item 3".to_string(),
        ];

        self.refresh_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[tokio::test]
async fn test_async_data_source_refresh_is_async() {
    // Verify that refresh can use .await
    let refresh_count = Arc::new(AtomicUsize::new(0));
    let mut data_source = AsyncTestDataSource {
        data: Vec::new(),
        refresh_count: refresh_count.clone(),
    };

    let ctx = TestContext {
        refresh_count: refresh_count.clone(),
    };

    // Call refresh - should complete asynchronously
    let result = data_source.refresh(&ctx).await;
    assert!(result.is_ok());

    // Verify data was updated
    assert_eq!(data_source.len(), 3);
    assert_eq!(data_source.get(0), Some(&"Item 1".to_string()));

    // Verify refresh was called
    assert_eq!(refresh_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn test_async_data_source_refresh_handles_errors() {
    struct ErrorDataSource {
        should_error: bool,
    }

    #[async_trait]
    impl DataSource for ErrorDataSource {
        type Row = String;

        fn len(&self) -> usize {
            0
        }

        fn get(&self, _index: usize) -> Option<&Self::Row> {
            None
        }

        async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
            if self.should_error {
                anyhow::bail!("Test error");
            }
            Ok(())
        }
    }

    let mut data_source = ErrorDataSource { should_error: true };
    let ctx = TestContext {
        refresh_count: Arc::new(AtomicUsize::new(0)),
    };

    let result = data_source.refresh(&ctx).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Test error"));
}

#[tokio::test]
async fn test_async_data_source_refresh_can_use_async_operations() {
    // Verify that refresh can perform async I/O operations
    let refresh_count = Arc::new(AtomicUsize::new(0));
    let mut data_source = AsyncTestDataSource {
        data: Vec::new(),
        refresh_count: refresh_count.clone(),
    };

    let ctx = TestContext {
        refresh_count: refresh_count.clone(),
    };

    // Start refresh
    let start = std::time::Instant::now();
    data_source.refresh(&ctx).await.unwrap();
    let elapsed = start.elapsed();

    // Verify it took at least the sleep duration (10ms)
    assert!(elapsed >= Duration::from_millis(10));

    // Verify data was updated
    assert_eq!(data_source.len(), 3);
}
