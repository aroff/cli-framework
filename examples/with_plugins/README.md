# `with_plugins`

Loads a sample plugin registry from **`plugin-registry.toml`** in the **current working directory** (the example creates this file on startup) and registers a **`builtin`** command.

## Run

From the repository root:

```bash
cargo run --example with_plugins
```

The demo writes `plugin-registry.toml` in the directory you run from. Remove it afterward if you do not need it.

## See also

README [Plugin system](../../README.md#plugin-system).
