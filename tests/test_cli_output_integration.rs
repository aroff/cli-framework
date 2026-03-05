//! Integration tests for CLI output utilities
//!
//! Tests that all CLI output utilities work together correctly.

use serde::Serialize;
use cli_framework::cli_output::{format_json, format_message, format_table, ColumnDef, GridData};
use cli_framework::message::AppMessage;

#[derive(Clone, Debug, Serialize)]
struct User {
    name: String,
    email: String,
    role: String,
}

#[test]
fn test_integration_table_and_json() {
    let users = vec![
        User {
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
            role: "Admin".to_string(),
        },
        User {
            name: "Bob".to_string(),
            email: "bob@example.com".to_string(),
            role: "User".to_string(),
        },
    ];

    // Format as table
    let grid = GridData {
        rows: users.clone(),
        columns: vec![
            ColumnDef {
                name: "Name".to_string(),
                width_hint: None,
                alignment: None,
            },
            ColumnDef {
                name: "Email".to_string(),
                width_hint: None,
                alignment: None,
            },
            ColumnDef {
                name: "Role".to_string(),
                width_hint: None,
                alignment: None,
            },
        ],
        row_headers: None,
    };

    let table_result = format_table(&grid);
    assert!(table_result.is_ok());
    let table = table_result.unwrap();
    assert!(table.contains("Name"));
    assert!(table.contains("Alice"));

    // Format as JSON
    let json_result = format_json(&users);
    assert!(json_result.is_ok());
    let json = json_result.unwrap();
    assert!(json.contains("\"name\""));
    assert!(json.contains("Alice"));
}

#[test]
fn test_integration_messages() {
    // Test message formatting
    let info = AppMessage::info("Operation completed");
    let formatted = format_message(&info);
    assert!(formatted.contains("ℹ") || formatted.contains("[INFO]"));
    assert!(formatted.contains("Operation completed"));

    let error = AppMessage::error("Operation failed");
    let formatted = format_message(&error);
    assert!(formatted.contains("✗") || formatted.contains("[ERROR]"));
    assert!(formatted.contains("Operation failed"));
}

#[test]
fn test_integration_all_utilities() {
    // Test that all utilities can be used together
    let user = User {
        name: "Test User".to_string(),
        email: "test@example.com".to_string(),
        role: "User".to_string(),
    };

    // Table
    let grid = GridData {
        rows: vec![user.clone()],
        columns: vec![ColumnDef {
            name: "Name".to_string(),
            width_hint: None,
            alignment: None,
        }],
        row_headers: None,
    };
    assert!(format_table(&grid).is_ok());

    // JSON
    assert!(format_json(&user).is_ok());

    // Messages
    let msg = AppMessage::info("Test message");
    assert!(!format_message(&msg).is_empty());
}
