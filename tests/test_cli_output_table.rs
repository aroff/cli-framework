//! Tests for table formatting functionality
//!
//! Tests for User Story 1: Display Tabular Data in CLI Output

use cli_framework::cli_output::table::{format_table, Alignment, ColumnDef, GridData};

#[derive(Clone, Debug, serde::Serialize)]
struct User {
    name: String,
    email: String,
    role: String,
}

#[test]
fn test_format_table_basic() {
    let grid = GridData {
        rows: vec![
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
        ],
        columns: vec![
            ColumnDef {
                name: "Name".to_string(),
                width_hint: None,
                alignment: Some(Alignment::Left),
            },
            ColumnDef {
                name: "Email".to_string(),
                width_hint: None,
                alignment: Some(Alignment::Left),
            },
            ColumnDef {
                name: "Role".to_string(),
                width_hint: None,
                alignment: Some(Alignment::Left),
            },
        ],
        row_headers: None,
    };

    let result = format_table(&grid);
    assert!(result.is_ok());
    let table = result.unwrap();
    assert!(table.contains("Name"));
    assert!(table.contains("Email"));
    assert!(table.contains("Role"));
    assert!(table.contains("Alice"));
    assert!(table.contains("Bob"));
}

#[test]
fn test_format_table_empty() {
    let grid: GridData<User> = GridData {
        rows: vec![],
        columns: vec![ColumnDef {
            name: "Name".to_string(),
            width_hint: None,
            alignment: None,
        }],
        row_headers: None,
    };

    let result = format_table(&grid);
    assert!(result.is_ok());
    let table = result.unwrap();
    assert!(table.contains("(empty)") || table.contains("empty"));
}

#[test]
fn test_format_table_missing_values() {
    #[derive(Clone, Debug, serde::Serialize)]
    struct UserWithOptional {
        name: String,
        email: Option<String>,
        role: String,
    }

    let grid = GridData {
        rows: vec![
            UserWithOptional {
                name: "Alice".to_string(),
                email: Some("alice@example.com".to_string()),
                role: "Admin".to_string(),
            },
            UserWithOptional {
                name: "Bob".to_string(),
                email: None, // Missing value
                role: "User".to_string(),
            },
        ],
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

    let result = format_table(&grid);
    assert!(result.is_ok());
    // Should handle missing values gracefully
}

#[test]
fn test_format_table_multiline() {
    let grid = GridData {
        rows: vec![User {
            name: "Alice".to_string(),
            email: "alice@example.com\nalice@work.com".to_string(), // Multi-line
            role: "Admin".to_string(),
        }],
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

    let result = format_table(&grid);
    assert!(result.is_ok());
    let table = result.unwrap();
    // Multi-line values should be wrapped within cells
    assert!(table.contains("alice@example.com"));
    assert!(table.contains("alice@work.com"));
}

#[test]
fn test_format_table_tui_mode() {
    // TUI mode should truncate with ellipsis when width exceeded
    // This is a basic test - full TUI mode testing requires terminal interaction
    let grid = GridData {
        rows: vec![User {
            name: "Very Long Name That Exceeds Normal Width".to_string(),
            email: "email@example.com".to_string(),
            role: "Admin".to_string(),
        }],
        columns: vec![
            ColumnDef {
                name: "Name".to_string(),
                width_hint: Some(20), // Constrained width
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

    let result = format_table(&grid);
    assert!(result.is_ok());
}

#[test]
fn test_format_table_cli_mode() {
    // CLI mode should output full table without truncation
    let grid = GridData {
        rows: vec![User {
            name: "Very Long Name That Should Not Be Truncated".to_string(),
            email: "email@example.com".to_string(),
            role: "Admin".to_string(),
        }],
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

    let result = format_table(&grid);
    assert!(result.is_ok());
    let table = result.unwrap();
    // In CLI mode, full content should be present
    assert!(table.contains("Very Long Name That Should Not Be Truncated"));
}

#[test]
fn test_format_table_wide_table() {
    // Test table with up to 200 characters wide (SC-008)
    let grid = GridData {
        rows: (0..10)
            .map(|i| User {
                name: format!("User{}", i),
                email: format!("user{}@example.com", i),
                role: "User".to_string(),
            })
            .collect(),
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

    let result = format_table(&grid);
    assert!(result.is_ok());
    let table = result.unwrap();
    // Table should be readable and properly formatted
    assert!(!table.is_empty());
}

#[test]
fn test_format_table_performance() {
    // Test performance with 1000 rows (SC-009)
    let grid = GridData {
        rows: (0..1000)
            .map(|i| User {
                name: format!("User{}", i),
                email: format!("user{}@example.com", i),
                role: "User".to_string(),
            })
            .collect(),
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

    let start = std::time::Instant::now();
    let result = format_table(&grid);
    let duration = start.elapsed();

    assert!(result.is_ok());
    // Guard against catastrophically slow implementations (SC-009).
    // 500ms accommodates debug builds on shared CI runners; local runs are typically <5ms.
    assert!(
        duration.as_millis() < 500,
        "Table formatting took {}ms, expected <500ms",
        duration.as_millis()
    );
}
