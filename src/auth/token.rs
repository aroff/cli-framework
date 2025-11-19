//! Token management for authentication
//!
//! Provides token storage and validation helpers

use std::time::{SystemTime, Duration};

/// Token manager for handling authentication tokens
pub struct TokenManager {
    token: Option<String>,
    expires_at: Option<SystemTime>,
}

impl TokenManager {
    /// Create a new token manager
    pub fn new() -> Self {
        Self {
            token: None,
            expires_at: None,
        }
    }

    /// Set a token with optional expiration
    pub fn set_token(&mut self, token: String, expires_in: Option<Duration>) {
        self.token = Some(token);
        self.expires_at = expires_in.map(|dur| SystemTime::now() + dur);
    }

    /// Get the current token if valid
    pub fn get_token(&self) -> Option<&str> {
        if let Some(ref token) = self.token {
            if let Some(expires) = self.expires_at {
                if SystemTime::now() < expires {
                    Some(token.as_str())
                } else {
                    None // Token expired
                }
            } else {
                Some(token.as_str()) // No expiration
            }
        } else {
            None
        }
    }

    /// Check if token is valid
    pub fn is_valid(&self) -> bool {
        self.get_token().is_some()
    }

    /// Clear the token
    pub fn clear(&mut self) {
        self.token = None;
        self.expires_at = None;
    }
}

impl Default for TokenManager {
    fn default() -> Self {
        Self::new()
    }
}
