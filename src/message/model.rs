//! AppMessage and AppMessageKind models
//!
//! Provides user-visible messages with different severity levels and detail levels.

/// Kind of application message
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMessageKind {
    /// Informational message
    Info,
    /// Warning message
    Warning,
    /// Error message
    Error,
}

/// User-visible message with short and detailed text
#[derive(Debug, Clone)]
pub struct AppMessage {
    /// Message kind (info, warning, error)
    pub kind: AppMessageKind,
    /// One-line text for status bar
    pub short: String,
    /// Optional detailed text for modal
    pub details: Option<String>,
}

impl AppMessage {
    /// Create a new info message
    pub fn info(short: impl Into<String>) -> Self {
        Self {
            kind: AppMessageKind::Info,
            short: short.into(),
            details: None,
        }
    }

    /// Create a new warning message
    pub fn warning(short: impl Into<String>) -> Self {
        Self {
            kind: AppMessageKind::Warning,
            short: short.into(),
            details: None,
        }
    }

    /// Create a new error message
    pub fn error(short: impl Into<String>) -> Self {
        Self {
            kind: AppMessageKind::Error,
            short: short.into(),
            details: None,
        }
    }

    /// Add detailed text to the message
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}
