//! Plugin registry management
//!
//! Handles loading and managing plugin registries from TOML configuration files.

use crate::plugin::PluginManifest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Plugin registry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRegistryConfig {
    /// Registry metadata
    pub metadata: RegistryMetadata,
    /// Plugin configurations
    pub plugins: HashMap<String, PluginEntry>,
}

/// Registry metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryMetadata {
    /// Registry name
    pub name: String,
    /// Registry version
    pub version: String,
    /// Description
    pub description: Option<String>,
}

/// Individual plugin entry in registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    /// Plugin name
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Path to plugin manifest file
    pub manifest_path: String,
    /// Whether the plugin is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Plugin priority (higher = loaded first)
    #[serde(default)]
    pub priority: i32,
}

fn default_enabled() -> bool {
    true
}

impl PluginRegistryConfig {
    /// Load registry from TOML file
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: PluginRegistryConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save registry to TOML file
    pub async fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        tokio::fs::write(path, content).await?;
        Ok(())
    }

    /// Get enabled plugins sorted by priority
    pub fn get_enabled_plugins(&self) -> Vec<(&String, &PluginEntry)> {
        let mut plugins: Vec<_> = self
            .plugins
            .iter()
            .filter(|(_, entry)| entry.enabled)
            .collect();

        plugins.sort_by(|a, b| b.1.priority.cmp(&a.1.priority));
        plugins
    }

    /// Add a plugin to the registry
    pub fn add_plugin(&mut self, id: String, entry: PluginEntry) {
        self.plugins.insert(id, entry);
    }

    /// Remove a plugin from the registry
    pub fn remove_plugin(&mut self, id: &str) -> bool {
        self.plugins.remove(id).is_some()
    }

    /// Enable or disable a plugin
    pub fn set_plugin_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(entry) = self.plugins.get_mut(id) {
            entry.enabled = enabled;
            true
        } else {
            false
        }
    }
}

/// Default registry configuration
impl Default for PluginRegistryConfig {
    fn default() -> Self {
        Self {
            metadata: RegistryMetadata {
                name: "CLI Framework Plugins".to_string(),
                version: "1.0.0".to_string(),
                description: Some("Plugin registry for CLI Framework applications".to_string()),
            },
            plugins: HashMap::new(),
        }
    }
}

fn validate_manifest_path(
    manifest_path: &std::path::Path,
    plugin_root: &std::path::Path,
) -> anyhow::Result<std::path::PathBuf> {
    let canonical_manifest = manifest_path.canonicalize().map_err(|e| {
        let err = anyhow::anyhow!(
            "PLUGIN_PATH_UNRESOLVED: cannot resolve manifest path '{}': {}",
            manifest_path.display(),
            e
        );
        tracing::error!(
            "Plugin path validation failed: cannot resolve manifest path '{}': {}",
            manifest_path.display(),
            e
        );
        err
    })?;
    let canonical_root = plugin_root.canonicalize().map_err(|e| {
        let err = anyhow::anyhow!(
            "PLUGIN_PATH_UNRESOLVED: cannot resolve plugin root '{}': {}",
            plugin_root.display(),
            e
        );
        tracing::error!(
            "Plugin path validation failed: cannot resolve plugin root '{}': {}",
            plugin_root.display(),
            e
        );
        err
    })?;
    if !canonical_manifest.starts_with(&canonical_root) {
        tracing::error!(
            "Plugin path escape detected: manifest '{}' is outside root '{}'",
            canonical_manifest.display(),
            canonical_root.display()
        );
        return Err(anyhow::anyhow!(
            "PLUGIN_PATH_ESCAPE: manifest path '{}' is outside plugin root '{}'",
            canonical_manifest.display(),
            canonical_root.display()
        ));
    }
    Ok(canonical_manifest)
}

/// Plugin registry manager
pub struct PluginRegistryManager {
    config_path: PathBuf,
    config: PluginRegistryConfig,
    loaded_manifests: HashMap<String, PluginManifest>,
}

impl PluginRegistryManager {
    /// Create a new registry manager
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            config: PluginRegistryConfig::default(),
            loaded_manifests: HashMap::new(),
        }
    }

    /// Load registry configuration and manifests
    pub fn load(&mut self) -> anyhow::Result<()> {
        // Load registry config
        if self.config_path.exists() {
            self.config = PluginRegistryConfig::from_file(&self.config_path)?;
        }

        // Load enabled plugin manifests
        let plugin_root = self
            .config_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine plugin root from config path"))?;
        for (plugin_id, entry) in self.config.get_enabled_plugins() {
            let manifest_path = PathBuf::from(&entry.manifest_path);
            match validate_manifest_path(&manifest_path, plugin_root) {
                Ok(validated_path) => match PluginManifest::from_file(&validated_path) {
                    Ok(manifest) => {
                        self.loaded_manifests.insert(plugin_id.clone(), manifest);
                    }
                    Err(e) => {
                        tracing::error!(plugin = %plugin_id, error = %e, "failed to load plugin");
                    }
                },
                Err(e) => {
                    tracing::error!(plugin = %plugin_id, error = %e, "plugin path validation failed");
                }
            }
        }

        Ok(())
    }

    /// Save registry configuration
    pub async fn save(&self) -> anyhow::Result<()> {
        self.config.save_to_file(&self.config_path).await
    }

    /// Get all loaded manifests
    pub fn manifests(&self) -> &HashMap<String, PluginManifest> {
        &self.loaded_manifests
    }

    /// Get a specific manifest
    pub fn get_manifest(&self, plugin_id: &str) -> Option<&PluginManifest> {
        self.loaded_manifests.get(plugin_id)
    }

    /// Register a new plugin
    pub async fn register_plugin(
        &mut self,
        plugin_id: String,
        name: String,
        version: String,
        manifest_path: String,
    ) -> anyhow::Result<()> {
        let entry = PluginEntry {
            name,
            version,
            manifest_path: manifest_path.clone(),
            enabled: true,
            priority: 0,
        };

        self.config.add_plugin(plugin_id.clone(), entry);

        // Try to load the manifest with path confinement validation
        let manifest_path_buf = PathBuf::from(manifest_path);
        let plugin_root = self
            .config_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine plugin root from config path"))?;
        let validated_path = validate_manifest_path(&manifest_path_buf, plugin_root)?;
        let manifest = PluginManifest::from_file(&validated_path)?;
        self.loaded_manifests.insert(plugin_id, manifest);

        Ok(())
    }

    /// Unregister a plugin
    pub fn unregister_plugin(&mut self, plugin_id: &str) -> bool {
        self.loaded_manifests.remove(plugin_id);
        self.config.remove_plugin(plugin_id)
    }

    /// Enable or disable a plugin
    pub fn set_plugin_enabled(&mut self, plugin_id: &str, enabled: bool) -> anyhow::Result<()> {
        if self.config.set_plugin_enabled(plugin_id, enabled) {
            if enabled {
                // Load the plugin if it's being enabled
                if let Some(entry) = self.config.plugins.get(plugin_id) {
                    let manifest_path = PathBuf::from(&entry.manifest_path);
                    let plugin_root = self.config_path.parent().ok_or_else(|| {
                        anyhow::anyhow!("Cannot determine plugin root from config path")
                    })?;
                    let validated_path = validate_manifest_path(&manifest_path, plugin_root)?;
                    let manifest = PluginManifest::from_file(&validated_path)?;
                    self.loaded_manifests
                        .insert(plugin_id.to_string(), manifest);
                }
            } else {
                // Unload the plugin if it's being disabled
                self.loaded_manifests.remove(plugin_id);
            }
            Ok(())
        } else {
            Err(anyhow::anyhow!("Plugin '{}' not found", plugin_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_registry_config() {
        let config = PluginRegistryConfig::default();
        assert_eq!(config.metadata.name, "CLI Framework Plugins");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("registry.toml");

        config.save_to_file(&config_path).await.unwrap();
        let loaded = PluginRegistryConfig::from_file(&config_path).unwrap();

        assert_eq!(loaded.metadata.name, config.metadata.name);
    }

    #[tokio::test]
    async fn test_plugin_registration() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("registry.toml");

        let mut manager = PluginRegistryManager::new(config_path.clone());

        // Create a test manifest
        let manifest_path = temp_dir.path().join("test-plugin.json");
        let manifest = PluginManifest {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Test plugin".to_string()),
            author: None,
            commands: vec![],
        };
        manifest.save_to_file(&manifest_path).await.unwrap();

        // Register the plugin
        manager
            .register_plugin(
                "test".to_string(),
                "Test Plugin".to_string(),
                "1.0.0".to_string(),
                manifest_path.to_string_lossy().to_string(),
            )
            .await
            .unwrap();

        assert!(manager.get_manifest("test").is_some());
        assert_eq!(manager.manifests().len(), 1);
    }
}
