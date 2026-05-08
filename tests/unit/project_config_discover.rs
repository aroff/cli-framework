use cli_framework::project_config::{
    find_file_upward, find_file_upward_with_options, DiscoverOptions, ProjectConfigError,
};
use std::fs;
use tempfile::TempDir;

fn make_nested(depth: usize) -> (TempDir, std::path::PathBuf) {
    let root = TempDir::new().unwrap();
    let mut current = root.path().to_path_buf();
    for i in 0..depth {
        current = current.join(format!("level{}", i));
        fs::create_dir_all(&current).unwrap();
    }
    (root, current)
}

#[test]
fn finds_file_in_start_dir() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("app.toml");
    fs::write(&config, "key = \"value\"").unwrap();

    let result = find_file_upward(dir.path(), "app.toml").unwrap();
    assert_eq!(result.root_dir, dir.path());
    assert_eq!(result.config_file, config);
}

#[test]
fn finds_file_in_parent() {
    let (root, leaf) = make_nested(2);
    let config = root.path().join("app.toml");
    fs::write(&config, "key = \"value\"").unwrap();

    let result = find_file_upward(&leaf, "app.toml").unwrap();
    assert_eq!(result.root_dir, root.path());
    assert_eq!(result.config_file, config);
}

#[test]
fn config_file_equals_root_dir_join_file_name() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.toml"), "x = 1").unwrap();

    let result = find_file_upward(dir.path(), "app.toml").unwrap();
    assert_eq!(result.config_file, result.root_dir.join("app.toml"));
}

// PC001
#[test]
fn not_found_returns_pc001() {
    let dir = TempDir::new().unwrap();
    let err = find_file_upward(dir.path(), "missing.toml").unwrap_err();
    assert!(matches!(err, ProjectConfigError::NotFound { .. }));
    let msg = err.to_string();
    assert!(msg.contains("PC001"));
    assert!(msg.contains("missing.toml"));
}

// PC004
#[test]
fn depth_exceeded_returns_pc004() {
    // file exists 2 levels up; max_depth = 1 should exceed
    let (root, leaf) = make_nested(2);
    fs::write(root.path().join("app.toml"), "x = 1").unwrap();

    let opts = DiscoverOptions {
        max_depth: Some(1),
        stop_at_marker: None,
    };
    let err = find_file_upward_with_options(&leaf, "app.toml", opts).unwrap_err();
    assert!(matches!(err, ProjectConfigError::DepthExceeded { .. }));
    let msg = err.to_string();
    assert!(msg.contains("PC004"));
}

// PC005
#[test]
fn invalid_file_name_with_slash_returns_pc005() {
    let dir = TempDir::new().unwrap();
    let err = find_file_upward(dir.path(), "sub/app.toml").unwrap_err();
    assert!(matches!(err, ProjectConfigError::InvalidFileName { .. }));
    let msg = err.to_string();
    assert!(msg.contains("PC005"));
}

#[test]
fn invalid_file_name_with_backslash_returns_pc005() {
    let dir = TempDir::new().unwrap();
    let err = find_file_upward(dir.path(), "sub\\app.toml").unwrap_err();
    assert!(matches!(err, ProjectConfigError::InvalidFileName { .. }));
}

// stop_at_marker: returns NotFound when marker seen before config file
#[test]
fn stop_at_marker_blocks_upward_walk() {
    let (root, leaf) = make_nested(2);
    // Place .git in the intermediate directory (level0)
    let mid = root.path().join("level0");
    fs::create_dir_all(mid.join(".git")).unwrap();
    // Place config file at root (above the marker)
    fs::write(root.path().join("app.toml"), "x = 1").unwrap();

    let opts = DiscoverOptions {
        max_depth: None,
        stop_at_marker: Some(".git".to_string()),
    };
    let err = find_file_upward_with_options(&leaf, "app.toml", opts).unwrap_err();
    assert!(matches!(err, ProjectConfigError::NotFound { .. }));
}

// stop_at_marker: file found before marker → success
#[test]
fn stop_at_marker_succeeds_when_file_found_first() {
    let (root, leaf) = make_nested(1);
    // Place config at start dir (leaf)
    fs::write(leaf.join("app.toml"), "x = 1").unwrap();
    // Place .git at root (above the leaf)
    fs::create_dir_all(root.path().join(".git")).unwrap();

    let opts = DiscoverOptions {
        max_depth: None,
        stop_at_marker: Some(".git".to_string()),
    };
    let result = find_file_upward_with_options(&leaf, "app.toml", opts).unwrap();
    assert_eq!(result.config_file, leaf.join("app.toml"));
}

// PC012 — all error variants implement std::error::Error
#[test]
fn error_is_std_error() {
    fn assert_error<E: std::error::Error>() {}
    assert_error::<ProjectConfigError>();
}

// Verify each variant's Display contains its code
#[test]
fn error_variants_contain_codes() {
    use std::path::PathBuf;

    let e = ProjectConfigError::NotFound {
        file_name: "x".into(),
        start: PathBuf::from("/tmp"),
    };
    assert!(e.to_string().contains("PC001"));

    let e = ProjectConfigError::ParseError {
        path: PathBuf::from("/tmp/x.toml"),
        source: toml::from_str::<toml::Value>("{{invalid").unwrap_err(),
    };
    assert!(e.to_string().contains("PC002"));

    let e = ProjectConfigError::IoError {
        path: PathBuf::from("/tmp/x.toml"),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "nope"),
    };
    assert!(e.to_string().contains("PC003"));

    let e = ProjectConfigError::DepthExceeded {
        max_depth: 3,
        start: PathBuf::from("/tmp"),
    };
    assert!(e.to_string().contains("PC004"));

    let e = ProjectConfigError::InvalidFileName {
        file_name: "sub/x.toml".into(),
    };
    assert!(e.to_string().contains("PC005"));
}
