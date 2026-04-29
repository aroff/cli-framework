# Testing with CliTestHarness

The `testkit` feature provides `CliTestHarness<C>` for deterministic, in-process testing of CLI applications.

## Enabling the feature

```toml
[dev-dependencies]
cli-framework = { path = ".", features = ["testkit", "clap-dispatch"] }
```

## Basic usage

```rust
use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::Command;
use cli_framework::testkit::CliTestHarness;
use std::sync::Arc;

struct MyCtx;
impl AppContext for MyCtx {}

#[tokio::test]
async fn test_version_output() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .build(MyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["myapp", "version"]).await;

    out.assert_exit_code(0);
    out.assert_stdout_contains("myapp 1.0.0");
}
```

## Asserting diagnostics

`TestOutput::assert_diagnostic_code(code)` panics if the given error code is not present in the captured stderr.

```rust
let out = harness.run(&["myapp", "bogus-cmd"]).await;
out.assert_diagnostic_code("E001");
```

## Normalizing output

`TestOutput::normalize()` returns a `NormalizedOutput` with ISO 8601 timestamps replaced by `<TIMESTAMP>`:

```rust
let normalized = out.normalize();
assert!(normalized.stdout.contains("<TIMESTAMP>") || !normalized.stdout.contains("T"));
```

## Environment overrides

Use `run_with_env` to set environment variables for a single invocation:

```rust
let out = harness
    .run_with_env(&["myapp", "deploy"], &[("ENV", "staging")])
    .await;
```

The variables are restored after the call.
