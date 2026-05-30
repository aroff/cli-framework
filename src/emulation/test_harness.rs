use super::mock_executor::MockExecutor;

pub struct TestHarness {
    pub executor: MockExecutor,
}

impl TestHarness {
    pub fn new() -> Self {
        Self {
            executor: MockExecutor::new(),
        }
    }
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}
