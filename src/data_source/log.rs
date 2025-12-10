//! LogDataSource trait and helpers for pushing logs to LogView
//!
//! Provides a pattern for applications to stream log lines to LogView widgets.

use crate::widget::LogView;
use std::sync::{Arc, Mutex};

/// Trait for sources that can provide log lines
///
/// Applications implement this trait to provide log streaming functionality.
/// The framework uses this to connect log sources to LogView widgets.
pub trait LogSource: Send + Sync {
    /// Get the next batch of log lines (non-blocking)
    ///
    /// Returns new lines since the last call, or empty if no new lines.
    fn poll_lines(&mut self) -> Vec<String>;

    /// Optional async poll for streaming sources
    ///
    /// Default implementation delegates to the sync `poll_lines`.
    fn poll_lines_async<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<String>> + Send + 'a>> {
        Box::pin(async move { self.poll_lines() })
    }

    /// Check if the log source is still active
    fn is_active(&self) -> bool;
}

/// Shared log buffer for thread-safe log streaming
///
/// Applications can push log lines from any thread, and LogView can consume them
/// from the main thread during rendering.
#[derive(Clone)]
pub struct SharedLogBuffer {
    lines: Arc<Mutex<VecDeque<String>>>,
    max_lines: usize,
}

impl SharedLogBuffer {
    /// Create a new shared log buffer
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: Arc::new(Mutex::new(VecDeque::new())),
            max_lines,
        }
    }

    /// Push a log line to the buffer
    pub fn push(&self, line: String) {
        if let Ok(mut lines) = self.lines.lock() {
            lines.push_back(line);
            while lines.len() > self.max_lines {
                lines.pop_front();
            }
        }
    }

    /// Push multiple log lines
    pub fn push_lines(&self, new_lines: Vec<String>) {
        if let Ok(mut lines) = self.lines.lock() {
            for line in new_lines {
                lines.push_back(line);
            }
            while lines.len() > self.max_lines {
                lines.pop_front();
            }
        }
    }

    /// Drain all pending lines (used by LogView to consume lines)
    pub fn drain(&self) -> Vec<String> {
        if let Ok(mut lines) = self.lines.lock() {
            let drained: Vec<String> = lines.drain(..).collect();
            drained
        } else {
            Vec::new()
        }
    }

    /// Get a clone of the buffer for reading
    pub fn clone_lines(&self) -> Vec<String> {
        if let Ok(lines) = self.lines.lock() {
            lines.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    /// Clear all lines
    pub fn clear(&self) {
        if let Ok(mut lines) = self.lines.lock() {
            lines.clear();
        }
    }

    /// Get the number of lines in the buffer
    pub fn len(&self) -> usize {
        if let Ok(lines) = self.lines.lock() {
            lines.len()
        } else {
            0
        }
    }
}

use std::collections::VecDeque;

/// Helper to connect a SharedLogBuffer to a LogView
///
/// Applications should call this periodically (e.g., in the render loop or event handler)
/// to transfer log lines from the buffer to the LogView.
pub fn sync_log_buffer_to_view(buffer: &SharedLogBuffer, view: &mut LogView) {
    let lines = buffer.drain();
    if !lines.is_empty() {
        view.add_lines(lines);
    }
}
