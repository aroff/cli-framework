// tests/integration/self_install_cli.rs
//! The `self` group from the outside (ADR 0080, phase 1).
//!
//! Part 1 checks registration through the builder: the group follows the
//! built-in namespace, is absent for `Deployment::Service`, and yields to an
//! application that owns `self`. Part 2 drives the real demo binary through
//! `self install --from-bootstrap`, `self status --json`, `doctor --json` and
//! `self uninstall` with a temporary home and bin dir.
//!
//! On Windows the receipt lands in the real `%LOCALAPPDATA%\cfw-self-install-demo`
//! (the known-folder API ignores environment overrides) and is removed by the
//! uninstall step. `CI=1` keeps the user `Path` untouched on every OS.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::Command;
use cli_framework::spec::command_tree::{CommandPath, CommandSpec};
use cli_framework::telemetry::Deployment;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;

struct Ctx;
impl AppContext for Ctx {}

fn path(parts: &[&str]) -> CommandPath {
    CommandPath::new(parts).unwrap()
}

fn options() -> cli_framework::self_install::SelfInstallOptions {
    cli_framework::self_install::SelfInstallOptions::github("aroff/demo")
}

// ── part 1: registration ─────────────────────────────────────────────────────

#[test]
fn group_is_registered_at_the_root_by_default() {
    let app = AppBuilder::new()
        .with_version("demo", "0.1.0")
        .with_self_install(options())
        .build(Ctx)
        .unwrap();
    let reg = app.command_registry();
    for leaf in ["install", "uninstall", "status"] {
        assert!(reg.resolve(&path(&["self", leaf])).is_some(), "self {leaf}");
    }
    assert!(reg.resolve(&path(&["doctor"])).is_some() || reg.get("doctor").is_some());
}

#[test]
fn group_follows_the_builtin_namespace() {
    let app = AppBuilder::new()
        .with_version("demo", "0.1.0")
        .with_builtin_command_namespace(&CommandPath::root_for("cli"))
        .with_self_install(options())
        .build(Ctx)
        .unwrap();
    let reg = app.command_registry();
    assert!(reg.resolve(&path(&["cli", "self", "install"])).is_some());
    assert!(reg.resolve(&path(&["self", "install"])).is_none());
    // The doctor command carrying the `install.*` checks is a built-in too.
    assert!(reg.resolve(&path(&["cli", "doctor"])).is_some());
    assert!(reg.get("doctor").is_none());
}

#[test]
fn an_app_owning_the_namespaced_doctor_keeps_it() {
    let own = Command {
        id: Arc::from("doctor"),
        spec: Arc::new(CommandSpec {
            summary: "the app's own doctor",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
    };
    let app = AppBuilder::new()
        .with_version("demo", "0.1.0")
        .with_builtin_command_namespace(&CommandPath::root_for("cli"))
        .register_command_at(&path(&["cli", "doctor"]), own)
        .unwrap()
        .with_self_install(options())
        .build(Ctx)
        .unwrap();
    let reg = app.command_registry();
    assert_eq!(
        reg.resolve(&path(&["cli", "doctor"])).unwrap().summary(),
        "the app's own doctor"
    );
    // No second doctor appears at the root.
    assert!(reg.get("doctor").is_none());
    assert!(reg.resolve(&path(&["cli", "self", "install"])).is_some());
}

#[test]
fn service_deployments_get_no_self_group() {
    let app = AppBuilder::new()
        .with_version("demo", "0.1.0")
        .with_deployment(Deployment::Service)
        .with_self_install(options())
        .build(Ctx)
        .unwrap();
    assert!(app
        .command_registry()
        .resolve(&path(&["self", "install"]))
        .is_none());
}

#[test]
fn an_app_owning_self_keeps_it() {
    let own = Command {
        id: Arc::from("self"),
        spec: Arc::new(CommandSpec {
            summary: "the app's own self command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
    };
    let app = AppBuilder::new()
        .with_version("demo", "0.1.0")
        .register_command(own)
        .unwrap()
        .with_self_install(options())
        .build(Ctx)
        .unwrap();
    let reg = app.command_registry();
    assert_eq!(
        reg.get("self").unwrap().summary(),
        "the app's own self command"
    );
    assert!(reg.resolve(&path(&["self", "install"])).is_none());
}

#[test]
fn invalid_options_fail_the_build() {
    let result = AppBuilder::new()
        .with_version("demo", "0.1.0")
        .with_self_install(cli_framework::self_install::SelfInstallOptions::http(
            "http://example.com/releases",
        ))
        .build(Ctx);
    assert!(result.is_err());
}

// ── part 2: the real binary ──────────────────────────────────────────────────

const DEMO: &str = env!("CARGO_BIN_EXE_cfw-self-install-demo");

struct Machine {
    _root: tempfile::TempDir,
    home: PathBuf,
    bin_dir: PathBuf,
}

impl Machine {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let bin_dir = root.path().join("bin");
        std::fs::create_dir_all(&home).unwrap();
        Machine {
            _root: root,
            home,
            bin_dir,
        }
    }

    fn run(&self, exe: &Path, args: &[&str]) -> Output {
        let mut cmd = std::process::Command::new(exe);
        cmd.args(args)
            .env("CI", "1")
            .env("HOME", &self.home)
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_STATE_HOME", self.home.join(".local/state"))
            .env_remove("XDG_BIN_HOME")
            .env_remove("CFW_DEMO_NAMESPACE");
        cmd.output().expect("run demo binary")
    }
}

fn ok(out: &Output, what: &str) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{what} failed ({}):\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn install_status_doctor_uninstall_round_trip() {
    let m = Machine::new();
    let exe_name = if cfg!(windows) {
        "cfw-self-install-demo.exe"
    } else {
        "cfw-self-install-demo"
    };
    let download_dir = m.home.join("Downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    let download = download_dir.join(exe_name);
    std::fs::copy(DEMO, &download).unwrap();

    let bin_dir = m.bin_dir.to_string_lossy().into_owned();
    let stdout = ok(
        &m.run(
            &download,
            &["self", "install", "--from-bootstrap", "--bin-dir", &bin_dir],
        ),
        "self install",
    );
    assert!(stdout.contains("installed to"), "{stdout}");
    assert!(stdout.contains("CI is set"), "{stdout}");
    let installed = m.bin_dir.join(exe_name);
    assert!(installed.is_file());
    assert!(!download.exists(), "--from-bootstrap moves the download");

    let status = ok(&m.run(&installed, &["self", "status", "--json"]), "status");
    let report: serde_json::Value = serde_json::from_str(status.trim()).unwrap();
    assert_eq!(report["method"], "script");
    assert_eq!(report["receipt"]["app"], "cfw-self-install-demo");
    assert_eq!(
        PathBuf::from(report["receipt"]["binary_path"].as_str().unwrap()),
        installed
    );

    let doctor = ok(&m.run(&installed, &["doctor", "--json"]), "doctor");
    let doctor: serde_json::Value = serde_json::from_str(doctor.trim()).unwrap();
    let ids: Vec<&str> = doctor["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["check_id"].as_str())
        .collect();
    for id in [
        "install.on_path",
        "install.shadowed",
        "install.receipt",
        "install.stale_files",
    ] {
        assert!(ids.contains(&id), "missing {id} in {ids:?}");
    }

    // A second copy placed over a foreign file is refused with SI002.
    let foreign_dir = m.home.join("foreign");
    std::fs::create_dir_all(&foreign_dir).unwrap();
    std::fs::write(foreign_dir.join(exe_name), b"not ours").unwrap();
    let foreign_dir_str = foreign_dir.to_string_lossy().into_owned();
    let refused = m.run(
        &installed,
        &[
            "self",
            "install",
            "--unmanaged",
            "--bin-dir",
            &foreign_dir_str,
        ],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("SI002"));

    let out = ok(&m.run(&installed, &["self", "uninstall"]), "uninstall");
    assert!(out.contains("uninstalled"), "{out}");
    // Windows deletes the running binary from a helper after exit.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while installed.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(!installed.exists(), "binary still present after uninstall");

    let again = m.run(DEMO.as_ref(), &["self", "uninstall"]);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("SI004"));
}
