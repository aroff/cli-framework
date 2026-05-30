use std::collections::HashMap;

#[cfg(feature = "emulation")]
use crate::command::Command;

pub struct MockExecutor {
    responses: HashMap<String, String>,
}

impl MockExecutor {
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
        }
    }

    pub fn expect(&mut self, command: &str, response: &str) {
        self.responses
            .insert(command.to_string(), response.to_string());
    }

    pub async fn execute(&self, command: &Command) -> Result<String, String> {
        self.responses
            .get(command.id)
            .cloned()
            .ok_or_else(|| format!("No mock response for command: {}", command.id))
    }
}

impl Default for MockExecutor {
    fn default() -> Self {
        Self::new()
    }
}
