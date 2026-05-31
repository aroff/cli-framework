use std::collections::HashMap;

pub struct Fixtures {
    data: HashMap<String, String>,
}

impl Fixtures {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: &str, value: &str) {
        self.data.insert(key.to_string(), value.to_string());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.data.get(key).map(|s| s.as_str())
    }
}

impl Default for Fixtures {
    fn default() -> Self {
        Self::new()
    }
}
