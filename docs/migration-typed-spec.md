# Migration Guide: Typed CommandSpec

**What this is:** The library today supports **both** styles: commands with **`spec: None` / `validator: None`** (simpler, legacy parse path) and commands with a full **`CommandSpec`** (typed arguments, stricter validation, richer help). This document is the **upgrade guide** when you choose the typed path. It is **not** a notice that the crate is legacy; most small apps can stay on **`spec: None`** until they need structured flags.

**Should you delete this file?** No, if you want a single place for **`register_command`?`**, feature flags (**`strict-types`**, **`strict-args`**, etc.), and **`CommandSpec`** examples. If you never adopt **`CommandSpec`**, you can ignore this file.

---
This guide covers migrating from optional-spec commands to the typed **`CommandSpec`** model (v0.3+).

## What changed

- `Command` now has two optional fields: `spec: Option<Arc<CommandSpec>>` and `validator: Option<Arc<dyn Fn(...)>>`.
- `AppBuilder::register_command()` now returns `Result<Self>` instead of `Self`.
- The clap parse path emits a `log::warn!` for commands without a spec (legacy-parse-path).

## Adding spec/validator to existing commands

**Before:**
```rust
Command {
    id: "deploy",
    summary: "Deploy to environment",
    syntax: Some("deploy --env <env>"),
    category: Some("deployment"),
    execute: Arc::new(|_ctx, args| Box::pin(async move {
        // ...
        Ok(())
    })),
}
```

**After (minimal — no behaviour change):**
```rust
Command {
    id: "deploy",
    summary: "Deploy to environment",
    syntax: Some("deploy --env <env>"),
    category: Some("deployment"),
    spec: None,
    validator: None,
    execute: Arc::new(|_ctx, args| Box::pin(async move {
        // ...
        Ok(())
    })),
}
```

**After (typed spec):**
```rust
Command {
    id: "deploy",
    summary: "Deploy to environment",
    syntax: Some("deploy --env <env>"),
    category: Some("deployment"),
    spec: None,
    validator: None,
    execute: Arc::new(|_ctx, args| Box::pin(async move {
        // ...
        Ok(())
    })),
}
.with_spec(CommandSpec {
    summary: "Deploy to an environment",
    long_about: Some("Deploys the current build to the specified environment."),
    args: vec![
        ArgSpec {
            name: "env",
            kind: ArgKind::Option,
            short: Some('e'),
            value_type: ArgValueType::Enum(vec!["staging", "production"]),
            cardinality: Cardinality::Required,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "Target environment",
        },
    ],
    ..Default::default()
})
```

## Updating register_command calls

`register_command` now returns `Result<Self>`, so callers must handle the result:

```rust
// Before:
AppBuilder::new()
    .register_command(my_cmd)
    .build(ctx)?;

// After:
AppBuilder::new()
    .register_command(my_cmd)?
    .build(ctx)?;

// Or with .unwrap() in tests:
AppBuilder::new()
    .register_command(my_cmd).unwrap()
    .build(ctx).unwrap();
```

## Feature flags

| Flag | Effect |
|------|--------|
| `strict-args` | Reject unknown flags on legacy (no-spec) commands |
| `strict-types` | Reject registration of commands without a CommandSpec |
| `legacy-arg-coercion` | Coerce bare `--flag` to `Bool(true)` on legacy path |
| `testkit` | Enable `CliTestHarness` for in-process test capture |
