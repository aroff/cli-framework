# Testing with CliTestHarness

**What this is:** Documentation for **automated tests** that you write in Rust and run with **`cargo test`** (unit and integration tests in your crate). Specifically, this file explains the optional **`CliTestHarness`** helper from **`cli-framework`**, which runs your **`App`** **in-process** so you can assert on stdout/stderr and exit behavior **without starting a separate OS process**.

**Who needs it:** App authors who want stable, fast regression tests around parsing, `--help`, and command dispatch.

**Who can skip it:** If you only use manual checks or subprocess-based tests, you do not need this harness.

---

The **`testkit`** feature exposes **`CliTestHarness<C>`** for deterministic CLI runs inside **`#[tokio::test]`** (or similar).

## Enabling the feature

```toml
[dev-dependencies]
cli-framework = { path = ".", features = ["testkit"] }
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
        // Opt-in: include a build-time git commit id when available.
        // Use `option_env!` so builds without the env var still compile.
        .with_git_sha_short(option_env!("VERGEN_GIT_SHA"))
        .build(MyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["myapp", "version"]).await;

    out.assert_exit_code(0);
    out.assert_stdout_contains("myapp 1.0.0");
}

#[tokio::test]
async fn test_version_flag_output() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .with_git_sha_short(Some("abc1234"))
        .build(MyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["myapp", "--version"]).await;

    out.assert_exit_code(0);
    out.assert_stdout_contains("myapp 1.0.0 (abc1234)");

    let out = harness.run(&["myapp", "-V"]).await;
    out.assert_exit_code(0);
    out.assert_stdout_contains("myapp 1.0.0 (abc1234)");
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

## Testing auth commands

When your app uses `with_token_provider`, supply a stub `TokenProvider` in tests — no OIDC server needed:

```rust
use cli_framework::auth::{AccessToken, AuthError, TokenProvider};
use cli_framework::testkit::CliTestHarness;
use std::sync::Arc;

struct AlwaysLoggedIn;

#[async_trait::async_trait]
impl TokenProvider for AlwaysLoggedIn {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        Ok(AccessToken::new("test-bearer".to_string(), None))
    }
    async fn invalidate(&self) {}
    async fn login(&self) -> Result<(), AuthError> { Ok(()) }
    async fn logout(&self) -> Result<(), AuthError> { Ok(()) }
}

#[tokio::test]
async fn auth_token_prints_bearer() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(AlwaysLoggedIn))
        .build(MyCtx)
        .unwrap();

    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "token"]).await;

    assert_eq!(out.exit_code(), 0);
    assert_eq!(out.stdout().trim(), "test-bearer");
}
```

Use `out.assert_diagnostic_code("AUTH003")` to assert that the not-authenticated error code is present when `token()` returns `AuthError::NotAuthenticated`.
