//! End-to-end `config` feature coverage driven through `CliTestHarness`,
//! mirroring `tests/integration/doctor_command.rs`'s pattern: build a real
//! `App`, register a plain `Command` that exercises the seam under test, and
//! assert on captured stdout / exit codes rather than internal call
//! sequences.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::Command;
use cli_framework::config::{ConfigOptions, VersionedConfig};
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::testkit::CliTestHarness;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tempfile::TempDir;

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct AppConfig {
    schema_version: u32,
    greeting: String,
}

impl VersionedConfig for AppConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

struct DummyCtx;
impl AppContext for DummyCtx {}

/// A command that reaches the framework-owned config handle purely through
/// `AppContext::opt_config_handle` — the object-safe seam every entry path
/// (not just a typed handler holding its own `Arc<ConfigStore<T>>`) can use,
/// e.g. a generic `doctor` check or `config` command group.
fn make_config_info_command() -> Command {
    Command {
        id: Arc::from("config-info"),
        spec: Arc::new(CommandSpec {
            summary: "Print the active config backend and value",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|ctx, _args| {
            Box::pin(async move {
                let handle = ctx
                    .opt_config_handle()
                    .ok_or_else(|| anyhow::anyhow!("no config handle wired"))?;
                ctx.framework_println(&format!("backend: {}", handle.backend_label()));
                ctx.framework_println(&format!("value: {}", handle.current_json()?));
                Ok(())
            })
        }),
    }
}

// User story 22 — `opt_config_handle` reaches the same backend label a
// `doctor` check would report, and `current_json` surfaces the resolved
// value without the caller knowing `AppConfig`.
#[tokio::test]
async fn opt_config_handle_reachable_from_a_plain_command() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.json");

    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .with_config_path(&path)
        .with_config::<AppConfig>(ConfigOptions::new(1))
        .register_command(make_config_info_command())
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "config-info"]).await;
    output.assert_exit_code(0);
    assert!(output.stdout().contains(&path.display().to_string()));
    assert!(output.stdout().contains("\"greeting\":\"\""));
}

// `with_config::<T>()` runs resolution once at `build()` time: a value
// already on disk before `build()` is picked up as the initial state.
#[tokio::test]
async fn build_resolves_an_existing_on_disk_value() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version": 1, "greeting": "already-here"}))
            .unwrap(),
    )
    .unwrap();

    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .with_config_path(&path)
        .with_config::<AppConfig>(ConfigOptions::new(1))
        .register_command(make_config_info_command())
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "config-info"]).await;
    assert!(output.stdout().contains("already-here"));
}

// `AppBuilder::build_with_config` hands back the typed resolved value
// alongside the built `App`, for a one-shot CLI that stores it as a plain
// field on its own context (spec 016 "Access" section).
#[test]
fn build_with_config_returns_typed_value_alongside_app() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version": 1, "greeting": "typed"})).unwrap(),
    )
    .unwrap();

    let (app, config) = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .with_config_path(&path)
        .with_config::<AppConfig>(ConfigOptions::new(1))
        .build_with_config::<DummyCtx, AppConfig>(DummyCtx)
        .unwrap();

    assert_eq!(config.greeting, "typed");
    drop(app);
}

// `App::config_store::<T>()` returns the same shared store backing
// `opt_config_handle` — reload through one is visible through the other,
// which is what makes typed subscription (user story 17) and the
// object-safe framework accessor consistent.
#[tokio::test]
async fn config_store_reload_is_visible_through_opt_config_handle() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.json");

    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .with_config_path(&path)
        .with_config::<AppConfig>(ConfigOptions::new(1))
        .register_command(make_config_info_command())
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let store = app.config_store::<AppConfig>().expect("store registered");

    // Out-of-band write, then reload through the *typed* store handle.
    std::fs::write(
        &path,
        serde_json::to_vec(
            &serde_json::json!({"schema_version": 1, "greeting": "via-typed-reload"}),
        )
        .unwrap(),
    )
    .unwrap();
    store.reload().unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "config-info"]).await;
    assert!(output.stdout().contains("via-typed-reload"));
}

// `App::config_store::<T>()` with a mismatched type returns `None` rather
// than panicking or silently returning an unrelated store.
#[test]
fn config_store_downcast_mismatch_returns_none() {
    #[derive(Default, Clone, Serialize, Deserialize)]
    struct OtherConfig {
        schema_version: u32,
    }
    impl VersionedConfig for OtherConfig {
        fn schema_version(&self) -> u32 {
            self.schema_version
        }
        fn set_schema_version(&mut self, v: u32) {
            self.schema_version = v;
        }
    }

    let dir = TempDir::new().unwrap();
    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .with_config_path(dir.path().join("cfg.json"))
        .with_config::<AppConfig>(ConfigOptions::new(1))
        .build(DummyCtx)
        .unwrap();

    assert!(app.config_store::<OtherConfig>().is_none());
    assert!(app.config_store::<AppConfig>().is_some());
}

// An app that never calls `with_config` gets no handle at all — the seam is
// fully opt-in, matching `opt_registry`'s "None for contexts that do not
// expose it" contract.
#[tokio::test]
async fn without_with_config_opt_config_handle_is_none() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .register_command(make_config_info_command())
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "config-info"]).await;
    // The command's own `ok_or_else` fires, surfacing as a runtime error (exit 1).
    output.assert_exit_code(1);
}

// Default backend wiring: an app that calls `with_config::<T>()` but neither
// `with_config_backend` nor `with_config_path` gets `FileBackend::for_app`.
// Redirect `$XDG_CONFIG_HOME` so this never touches the real user profile.
#[tokio::test]
async fn default_backend_is_file_backend_for_app() {
    let dir = TempDir::new().unwrap();
    let original = std::env::var("XDG_CONFIG_HOME").ok();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    let app = AppBuilder::new()
        .with_version("config-default-app", "1.0.0")
        .with_config::<AppConfig>(ConfigOptions::new(1))
        .register_command(make_config_info_command())
        .unwrap()
        .build(DummyCtx);

    match original {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }

    let app = app.unwrap();
    let expected_path = dir.path().join("config-default-app").join("config");
    let mut harness = CliTestHarness::new(app);
    let output = harness.run(&["myapp", "config-info"]).await;
    output.assert_exit_code(0);
    assert!(
        output
            .stdout()
            .contains(&expected_path.display().to_string()),
        "expected backend label to name {:?}, got:\n{}",
        expected_path,
        output.stdout()
    );
    // Nothing has been saved yet (only resolved/read at build time), so the
    // file itself is not expected to exist — only the *label* names where it
    // would be written, matching FileBackend's "absent = empty bytes" read
    // contract.
    assert!(!expected_path.exists());
}

// `ConfigOptions::default()` picks current_version 1 and JSON — the common
// case for a brand-new config type.
#[test]
fn config_options_default_is_version_one_json() {
    let dir = TempDir::new().unwrap();
    let (_app, config) = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .with_config_path(dir.path().join("cfg.json"))
        .with_config::<AppConfig>(ConfigOptions::default())
        .build_with_config::<DummyCtx, AppConfig>(DummyCtx)
        .unwrap();
    assert_eq!(config.schema_version, 1);
}

// `ConfigOptions::with_format` actually changes what `AppBuilder::with_config`
// wires up: the file on disk after a save is TOML text, not JSON.
#[test]
fn config_options_with_format_toml_writes_toml_on_disk() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg");

    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .with_config_path(&path)
        .with_config::<AppConfig>(
            ConfigOptions::new(1).with_format(cli_framework::config::ConfigFormat::Toml),
        )
        .build(DummyCtx)
        .unwrap();

    let store = app.config_store::<AppConfig>().unwrap();
    let mut cfg = (*store.current()).clone();
    cfg.greeting = "toml-via-options".to_string();
    store.save(&cfg).unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("toml-via-options"));
    assert!(!on_disk.trim_start().starts_with('{'));
}

// `ConfigOptions::with_migration` registers migrations that
// `AppBuilder::with_config` actually applies during `build()`'s one-time
// resolution — an on-disk document at an older version arrives migrated in
// the typed value `build_with_config` hands back.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
struct MigratedAppConfig {
    schema_version: u32,
    full_name: String,
}
impl VersionedConfig for MigratedAppConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, v: u32) {
        self.schema_version = v;
    }
}

#[test]
fn config_options_with_migration_applies_during_build() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version": 1, "name": "Ada"})).unwrap(),
    )
    .unwrap();

    let options = ConfigOptions::<MigratedAppConfig>::new(2).with_migration(1, |mut value| {
        if let serde_json::Value::Object(map) = &mut value {
            if let Some(name) = map.remove("name") {
                map.insert("full_name".to_string(), name);
            }
        }
        Ok(value)
    });

    let (_app, config) = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .with_config_path(&path)
        .with_config::<MigratedAppConfig>(options)
        .build_with_config::<DummyCtx, MigratedAppConfig>(DummyCtx)
        .unwrap();

    assert_eq!(config.full_name, "Ada");
    assert_eq!(config.schema_version, 2);
}
