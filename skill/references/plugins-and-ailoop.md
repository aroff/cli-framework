# Plugins and ailoop

Plugin registry and ailoop-core confirmation patterns. See [`skill/examples/with_plugins`](../examples/with_plugins/) and [`skill/examples/with_ailoop`](../examples/with_ailoop/).

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

## ailoop confirmation pattern

`ailoop-core` provides human-in-the-loop confirmation before risky operations:

```rust
use cli_framework::ailoop::AiloopClient;

// Inside an execute closure:
execute: Arc::new(|_ctx, args| Box::pin(async move {
    let target = args.get("target").unwrap_or("unknown");
    let ailoop = AiloopClient::new()?;
    let confirmed = ailoop
        .request_confirmation(
            &format!("Delete all data for {target}?"),
            Some("This action cannot be undone"),
        )
        .await?;
    if !confirmed {
        println!("Aborted.");
        return Ok(());
    }
    println!("Deleting {target}...");
    Ok(())
})),
```

## `with_ailoop` builder method

```rust
let builder = AppBuilder::new()
    .with_ailoop_channel("my-channel-id")?;
    // or: .with_ailoop_from_env()? — reads AILOOP_CHANNEL from environment
```
