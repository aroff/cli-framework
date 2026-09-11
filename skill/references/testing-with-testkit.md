# Testing with testkit

`CliTestHarness` for in-process CLI testing. See also [`docs/testing.md`](../../docs/testing.md).

## Feature gate

```toml
[dev-dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework", features = ["testkit"] }
```

Only use `testkit` in `[dev-dependencies]` or `#[cfg(test)]` contexts; it is not for production binaries.

## `CliTestHarness`

Runs the full CLI dispatch pipeline in-process without spawning a subprocess. Captures stdout and returns it as a `String`.

```rust
#[cfg(test)]
mod tests {
    use cli_framework::testkit::CliTestHarness;

    fn build_app() -> anyhow::Result<cli_framework::App<AppCtx>> {
        AppBuilder::new()
            .register_command(health_cmd())?
            .build(AppCtx)
    }

    #[tokio::test]
    async fn health_returns_ok() -> anyhow::Result<()> {
        let harness = CliTestHarness::new(build_app()?);
        let output = harness.run(&["health"]).await?;
        assert!(output.contains("ok"), "unexpected output: {output}");
        Ok(())
    }
}
```

## Patterns

Run with arguments:

```rust
let output = harness.run(&["deploy", "create", "--env", "staging"]).await?;
assert!(output.contains("deploying to staging"));
```

Assert exit error:

```rust
let result = harness.run(&["deploy", "create"]).await;
assert!(result.is_err(), "expected validation error for missing --env");
```

Telemetry-enabled apps: isolate the consent file and make the first-run notice
deterministic (see `telemetry.md`):

```rust
let app = AppBuilder::new()
    .with_version("demo", "0.0.0")
    .with_telemetry_config_dir(tmp.path())   // never the developer's real settings file
    .build(TestCtx)?;
let harness = CliTestHarness::new(app).with_interactive_stderr(true); // notice prints; default is false
```

## Feature-gated test suites

Cargo supports feature-gated test targets:

```toml
[[test]]
name = "integration_mytool"
path = "tests/integration/mytool.rs"
required-features = ["testkit"]
```

Run with: `cargo test --features testkit`.
