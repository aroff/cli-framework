# Plugins

Plugin registry patterns. See [`skill/examples/with_plugins`](../examples/with_plugins/).

## Plugin manifest format

Plugins are defined in TOML manifest files. Each plugin declares commands that are registered alongside built-ins when `PluginRegistryManager` loads the manifests:

```toml
# ~/.config/mytool/plugins/myplugin.toml
[plugin]
name = "myplugin"
version = "1.0.0"

[[commands]]
id = "greet"
summary = "Greet a user"
category = "misc"
```

## `PluginRegistryManager`

```rust
use cli_framework::plugin::PluginRegistryManager;

// In main, after builder setup:
let plugin_dir = dirs::config_dir()
    .map(|d| d.join("mytool").join("plugins"))
    .unwrap_or_default();

let mut builder = AppBuilder::new();
let manager = PluginRegistryManager::new(plugin_dir);
builder = manager.register_plugins(builder)?;
```

Plugin load failures for one plugin do not affect other registered plugins or built-in commands.
