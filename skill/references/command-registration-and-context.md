# Command registration and context

How `AppBuilder`, `AppContext`, and `Arc` executor patterns work together.

## `AppContext`

Implement `AppContext` on your context struct. No methods are required; the trait is a marker that holds shared state:

```rust
use cli_framework::prelude::*;

struct AppCtx {
    http: reqwest::Client,
    base_url: String,
}
impl AppContext for AppCtx {}
```

The context is wrapped in `Arc` internally; all `execute` closures receive `Arc<C>`.

## `AppBuilder` chain

```rust
let mut app = AppBuilder::new()
    .register_command(health_cmd())?
    .register_command(version_cmd())?
    .with_version("mytool", "1.0.0")
    .build(AppCtx { http: reqwest::Client::new(), base_url: "https://api.example.com".into() })?;
app.run().await
```

Steps:
1. `AppBuilder::new()` — creates builder with empty registry
2. `.register_command(cmd)?` — adds `Command` to root; returns `Err` if `id` conflicts
3. `.register_command_at(&path, cmd)?` — adds `Command` at a hierarchical path
4. `.with_version(name, ver)` — sets app name and version (used for MCP tool prefix and `--version` flag)
5. `.with_llm_from_env()?` — wires LLM provider from `LLM_PROVIDER` or `OPENAI_API_KEY`
6. `.build(ctx)?` — freezes registry, wraps context in `Arc`
7. `app.run().await` — parses `std::env::args()`, resolves command, calls `execute`

## `Arc` executor pattern

```rust
execute: Arc::new(|ctx, args| Box::pin(async move {
    let url = format!("{}/health", ctx.base_url);
    let resp = ctx.http.get(&url).send().await?;
    println!("{}", resp.status());
    Ok(())
})),
```

`ctx` is `Arc<C>`. Clone `Arc` fields into the async block rather than borrowing.

## Hierarchical dispatch

```rust
// registers at path: project/init
builder.register_command_at(
    &CommandPath::new(&["project", "init"])?,
    Command { id: "init", /* ... */ execute: Arc::new(|_ctx, _args| Box::pin(async move {
        println!("project initialized");
        Ok(())
    })) },
)?;
```

`CommandPath::new` validates that segments are non-empty and contain no `/`. Use `register_command_at` for all hierarchical paths; `register_command` registers at the root.
