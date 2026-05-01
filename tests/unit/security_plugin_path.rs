use cli_framework::plugin::{PluginManifest, PluginRegistryManager};
use tempfile::TempDir;

fn make_manifest() -> PluginManifest {
    PluginManifest {
        name: "test-plugin".to_string(),
        version: "1.0.0".to_string(),
        description: Some("Test plugin".to_string()),
        author: None,
        commands: vec![],
    }
}

// AC12: Non-existent path returns PLUGIN_PATH_UNRESOLVED
#[tokio::test]
async fn test_nonexistent_path_unresolved() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("registry.toml");
    let mut manager = PluginRegistryManager::new(config_path);

    let result = manager
        .register_plugin(
            "test".to_string(),
            "Test".to_string(),
            "1.0.0".to_string(),
            "/nonexistent/path/that/does/not/exist.json".to_string(),
        )
        .await;

    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("PLUGIN_PATH_UNRESOLVED"),
        "Expected PLUGIN_PATH_UNRESOLVED, got: {}",
        msg
    );
}

// AC11: Path outside plugin root returns PLUGIN_PATH_ESCAPE
#[tokio::test]
async fn test_path_outside_root_escape() {
    let root_dir = TempDir::new().unwrap();
    let other_dir = TempDir::new().unwrap();

    let config_path = root_dir.path().join("registry.toml");

    // Create a valid manifest in the OTHER directory (outside root)
    let manifest_path = other_dir.path().join("manifest.json");
    let manifest = make_manifest();
    manifest.save_to_file(&manifest_path).await.unwrap();

    let mut manager = PluginRegistryManager::new(config_path);

    let result = manager
        .register_plugin(
            "test".to_string(),
            "Test".to_string(),
            "1.0.0".to_string(),
            manifest_path.to_string_lossy().to_string(),
        )
        .await;

    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("PLUGIN_PATH_ESCAPE"),
        "Expected PLUGIN_PATH_ESCAPE, got: {}",
        msg
    );
}

// AC13: Valid path inside plugin root succeeds
#[tokio::test]
async fn test_valid_path_inside_root_succeeds() {
    let root_dir = TempDir::new().unwrap();
    let config_path = root_dir.path().join("registry.toml");

    // Create a valid manifest INSIDE the root directory
    let manifest_path = root_dir.path().join("test-plugin.json");
    let manifest = make_manifest();
    manifest.save_to_file(&manifest_path).await.unwrap();

    let mut manager = PluginRegistryManager::new(config_path);

    let result = manager
        .register_plugin(
            "test".to_string(),
            "Test Plugin".to_string(),
            "1.0.0".to_string(),
            manifest_path.to_string_lossy().to_string(),
        )
        .await;

    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
    assert!(manager.get_manifest("test").is_some());
}

// AC13: PluginManifest::from_file works on a valid path
#[tokio::test]
async fn test_plugin_manifest_from_file_succeeds() {
    let temp_dir = TempDir::new().unwrap();
    let manifest_path = temp_dir.path().join("manifest.json");
    let manifest = make_manifest();
    manifest.save_to_file(&manifest_path).await.unwrap();

    let loaded = PluginManifest::from_file(&manifest_path);
    assert!(loaded.is_ok());
    assert_eq!(loaded.unwrap().name, "test-plugin");
}
