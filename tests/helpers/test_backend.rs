//! TestBackend helper utilities for headless TUI testing
//!
//! This module provides utilities for testing TUI rendering logic without
//! requiring a physical terminal. Uses `ratatui::backend::TestBackend` to
//! assert buffer content (characters and styles) at specific coordinates.

use ratatui::backend::TestBackend;
use ratatui::Buffer;

/// Creates a TestBackend with the specified dimensions
pub fn create_test_backend(width: u16, height: u16) -> TestBackend {
    TestBackend::new(width, height)
}

/// Asserts that a character at a specific position matches the expected value
pub fn assert_char_at(buffer: &Buffer, x: u16, y: u16, expected: char) {
    let cell = buffer.get(x, y);
    assert_eq!(
        cell.symbol().chars().next().unwrap_or(' '),
        expected,
        "Character at ({}, {}) does not match. Expected '{}', got '{}'",
        x,
        y,
        expected,
        cell.symbol()
    );
}

/// Asserts that a string appears at a specific position in the buffer
pub fn assert_string_at(buffer: &Buffer, x: u16, y: u16, expected: &str) {
    let mut actual = String::new();
    for i in 0..expected.len() {
        if let Some(cell) = buffer.get(x + i as u16, y) {
            actual.push_str(cell.symbol());
        }
    }
    assert_eq!(
        actual.trim(),
        expected.trim(),
        "String at ({}, {}) does not match. Expected '{}', got '{}'",
        x,
        y,
        expected,
        actual
    );
}

/// Asserts that a region of the buffer contains the expected text
pub fn assert_region_contains(buffer: &Buffer, x: u16, y: u16, width: u16, height: u16, expected: &str) {
    let mut actual = String::new();
    for row in y..(y + height) {
        for col in x..(x + width) {
            if let Some(cell) = buffer.get(col, row) {
                actual.push_str(cell.symbol());
            }
        }
        actual.push('\n');
    }
    assert!(
        actual.contains(expected),
        "Region ({}, {}) {}x{} does not contain '{}'. Actual content:\n{}",
        x,
        y,
        width,
        height,
        expected,
        actual
    );
}

