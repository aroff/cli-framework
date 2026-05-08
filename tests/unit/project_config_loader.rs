use cli_framework::project_config::{load_toml_file, load_toml_str, ProjectConfigError};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[derive(serde::Deserialize, Debug, PartialEq)]
struct TestConfig {
    name: String,
    value: u32,
}

// PC005 (AC 5) — load_toml_file returns Ok(T) for valid TOML matching T
#[test]
fn load_toml_file_ok_for_valid_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.toml");
    fs::write(&path, r#"name = "hello"\nvalue = 42"#.replace("\\n", "\n")).unwrap();

    let cfg: TestConfig = load_toml_file(&path).unwrap();
    assert_eq!(cfg.name, "hello");
    assert_eq!(cfg.value, 42);
}

// PC006 (AC 6) — load_toml_file returns ParseError for invalid TOML
#[test]
fn load_toml_file_parse_error_for_bad_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.toml");
    fs::write(&path, "{{not valid toml").unwrap();

    let err = load_toml_file::<TestConfig>(&path).unwrap_err();
    assert!(matches!(err, ProjectConfigError::ParseError { .. }));
    assert!(err.to_string().contains("PC002"));
}

// PC006 — load_toml_file returns ParseError for type mismatch
#[test]
fn load_toml_file_parse_error_for_type_mismatch() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("mismatch.toml");
    // 'value' is a string but the struct expects u32
    fs::write(&path, "name = \"x\"\nvalue = \"not_a_number\"").unwrap();

    let err = load_toml_file::<TestConfig>(&path).unwrap_err();
    assert!(matches!(err, ProjectConfigError::ParseError { .. }));
}

// PC007 (AC 7) — load_toml_file returns IoError when path does not exist
#[test]
fn load_toml_file_io_error_for_missing_file() {
    let path = PathBuf::from("/tmp/__nonexistent_cli_framework_test_file__.toml");
    let err = load_toml_file::<TestConfig>(&path).unwrap_err();
    assert!(matches!(err, ProjectConfigError::IoError { .. }));
    assert!(err.to_string().contains("PC003"));
}

// load_toml_str — valid TOML string
#[test]
fn load_toml_str_ok() {
    let content = "name = \"world\"\nvalue = 7";
    let cfg: TestConfig = load_toml_str(content, &PathBuf::from("virtual.toml")).unwrap();
    assert_eq!(cfg.name, "world");
    assert_eq!(cfg.value, 7);
}

// load_toml_str — invalid TOML string returns ParseError
#[test]
fn load_toml_str_parse_error() {
    let err = load_toml_str::<TestConfig>("[[invalid", &PathBuf::from("virtual.toml")).unwrap_err();
    assert!(matches!(err, ProjectConfigError::ParseError { .. }));
}

// PC012 — ProjectConfigError implements std::error::Error
#[test]
fn error_is_std_error() {
    fn assert_error<E: std::error::Error>() {}
    assert_error::<ProjectConfigError>();
}
