// tests/unit/self_install.rs
//! Self-install operations (ADR 0080, phase 1) over a hand-built
//! [`InstallEnv`]: temporary home, state and bin directories, and on Windows
//! a scratch registry key. Nothing here reads or writes the real home, PATH
//! or `HKCU\Environment`.

use cli_framework::self_install::{
    default_bin_dir, env_var_prefix, fish_env_file_contents, infer_method_from_path, install,
    path_decision, posix_env_file_contents, purge_roots, rc_candidates, receipt_path, stale_files,
    status, uninstall, InstallEnv, InstallMethod, InstallReceipt, InstallRequest, Os, PathDecision,
    PathModification, SelfInstallError, SelfInstallOptions,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const APP: &str = "demoapp";

struct Sandbox {
    _root: TempDir,
    home: PathBuf,
    downloads: PathBuf,
    env: InstallEnv,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        let downloads = root.path().join("downloads");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&downloads).unwrap();
        let os = Os::current();
        let exe = downloads.join(if os.is_windows() {
            format!("{APP}.exe")
        } else {
            APP.to_string()
        });
        std::fs::write(&exe, b"#!/bin/sh\necho demo\n").unwrap();
        let mut vars = BTreeMap::new();
        vars.insert("PATH".to_string(), String::new());
        let env = InstallEnv {
            app: APP.to_string(),
            version: "1.2.3".to_string(),
            os,
            home: Some(home.clone()),
            vars,
            current_exe: exe,
            stdout_is_terminal: true,
            is_root: false,
            state_root: Some(root.path().join("state")),
            config_root: Some(root.path().join("config")),
            data_root: Some(root.path().join("data")),
            user_path_key: format!(
                "Software\\cli-framework-test\\{}",
                uuid::Uuid::new_v4().simple()
            ),
            uninstall_key_root: format!(
                "Software\\cli-framework-test\\uninstall-{}",
                uuid::Uuid::new_v4().simple()
            ),
            completion_command: None,
            self_invocation: format!("{APP} self"),
        };
        Sandbox {
            _root: root,
            home,
            downloads,
            env,
        }
    }

    fn bin_dir(&self) -> PathBuf {
        self.home.join(".local").join("bin")
    }

    fn binary(&self) -> PathBuf {
        self.bin_dir().join(if self.env.os.is_windows() {
            format!("{APP}.exe")
        } else {
            APP.to_string()
        })
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
            let _ = hkcu.delete_subkey_all(&self.env.user_path_key);
        }
    }
}

fn options() -> SelfInstallOptions {
    SelfInstallOptions::github("aroff/demoapp").completions(false)
}

fn req() -> InstallRequest {
    InstallRequest::default()
}

// ── options ──────────────────────────────────────────────────────────────────

#[test]
fn options_validate_rejects_bad_sources() {
    assert!(SelfInstallOptions::github("aroff/demoapp")
        .validate()
        .is_ok());
    assert!(SelfInstallOptions::github("demoapp").validate().is_err());
    assert!(SelfInstallOptions::http("http://example.com/releases")
        .validate()
        .is_err());
    assert!(SelfInstallOptions::http("https://example.com/releases")
        .validate()
        .is_ok());
    assert!(SelfInstallOptions::http("http://127.0.0.1:8000")
        .validate()
        .is_ok());
    assert!(SelfInstallOptions::github("aroff/demoapp")
        .asset_template("{app}.tar.gz")
        .validate()
        .is_err());
}

#[test]
fn asset_name_follows_the_release_contract() {
    let o = SelfInstallOptions::github("aroff/demoapp");
    assert_eq!(
        o.asset_name("demoapp", "v1.2.3", "x86_64-unknown-linux-musl"),
        "demoapp-x86_64-unknown-linux-musl.tar.gz"
    );
    assert_eq!(
        o.asset_name("demoapp", "1.2.3", "aarch64-pc-windows-msvc"),
        "demoapp-aarch64-pc-windows-msvc.zip"
    );
}

#[test]
fn env_var_prefix_is_shell_safe() {
    assert_eq!(env_var_prefix("my-app.cli"), "MY_APP_CLI");
}

// ── layout and method ────────────────────────────────────────────────────────

#[test]
fn default_bin_dir_is_local_bin_or_xdg_bin_home() {
    let mut sb = Sandbox::new();
    assert_eq!(default_bin_dir(&sb.env), Some(sb.bin_dir()));
    let xdg = sb.home.join("xdg-bin");
    sb.env
        .vars
        .insert("XDG_BIN_HOME".into(), xdg.to_string_lossy().into_owned());
    let expected = if sb.env.os.is_windows() {
        sb.bin_dir()
    } else {
        xdg
    };
    assert_eq!(default_bin_dir(&sb.env), Some(expected));
    sb.env
        .vars
        .insert("XDG_BIN_HOME".into(), "relative/bin".into());
    assert_eq!(default_bin_dir(&sb.env), Some(sb.bin_dir()));
}

#[test]
fn package_manager_paths_are_recognised() {
    let cases = [
        (
            "/opt/homebrew/Cellar/demoapp/1.0/bin/demoapp",
            Some(InstallMethod::Homebrew),
        ),
        (
            "/home/linuxbrew/.linuxbrew/bin/demoapp",
            Some(InstallMethod::Homebrew),
        ),
        (
            "C:/Users/me/scoop/apps/demoapp/current/demoapp.exe",
            Some(InstallMethod::Scoop),
        ),
        ("/home/me/.cargo/bin/demoapp", Some(InstallMethod::Cargo)),
        ("/home/me/.local/bin/demoapp", None),
    ];
    for (path, expected) in cases {
        assert_eq!(infer_method_from_path(Path::new(path)), expected, "{path}");
    }
}

// ── PATH decision and shell files ────────────────────────────────────────────

#[test]
fn path_decision_skips_ci_non_tty_and_flags() {
    let mut sb = Sandbox::new();
    let bin = sb.bin_dir();
    assert_eq!(path_decision(&sb.env, &req(), &bin), PathDecision::Edit);

    let no_modify = InstallRequest {
        no_modify_path: true,
        ..req()
    };
    assert!(matches!(
        path_decision(&sb.env, &no_modify, &bin),
        PathDecision::Skip(_)
    ));

    sb.env.stdout_is_terminal = false;
    assert!(matches!(
        path_decision(&sb.env, &req(), &bin),
        PathDecision::Skip(_)
    ));
    sb.env.stdout_is_terminal = true;

    sb.env.vars.insert("CI".into(), "true".into());
    assert_eq!(
        path_decision(&sb.env, &req(), &bin),
        PathDecision::Skip("CI is set".into())
    );
    sb.env.vars.insert("CI".into(), String::new());

    std::fs::create_dir_all(&bin).unwrap();
    let sep = if sb.env.os.is_windows() { ";" } else { ":" };
    sb.env.vars.insert(
        "PATH".into(),
        format!("/usr/bin{sep}{}", bin.to_string_lossy()),
    );
    assert_eq!(
        path_decision(&sb.env, &req(), &bin),
        PathDecision::AlreadyOnPath
    );
}

#[test]
fn env_files_match_uv_shape() {
    let posix = posix_env_file_contents("$HOME/.local/bin");
    assert!(posix.starts_with("#!/bin/sh\n"));
    assert!(posix.contains("*:\"$HOME/.local/bin\":*)"));
    assert!(posix.contains("export PATH=\"$HOME/.local/bin:$PATH\""));
    let fish = fish_env_file_contents("$HOME/.local/bin");
    assert!(fish.contains("set -x PATH \"$HOME/.local/bin\" $PATH"));
}

#[test]
fn rc_candidates_only_lists_existing_files_and_prefers_zshenv() {
    let sb = Sandbox::new();
    assert!(rc_candidates(&sb.home, None).is_empty());
    std::fs::write(sb.home.join(".bashrc"), "").unwrap();
    std::fs::write(sb.home.join(".zshrc"), "").unwrap();
    assert_eq!(
        rc_candidates(&sb.home, None),
        vec![sb.home.join(".bashrc"), sb.home.join(".zshrc")]
    );
    std::fs::write(sb.home.join(".zshenv"), "").unwrap();
    assert_eq!(
        rc_candidates(&sb.home, None),
        vec![sb.home.join(".bashrc"), sb.home.join(".zshenv")]
    );
}

// ── install / status / uninstall ─────────────────────────────────────────────

#[test]
fn install_places_binary_writes_receipt_and_uninstall_reverses_it() {
    let sb = Sandbox::new();
    if !sb.env.os.is_windows() {
        std::fs::write(sb.home.join(".bashrc"), "# existing\n").unwrap();
    }

    let out = install(&sb.env, &options(), &req()).expect("install");
    assert_eq!(out.binary_path, sb.binary());
    assert!(sb.binary().is_file());
    assert!(sb.env.current_exe.is_file(), "copy mode keeps the source");
    assert_eq!(out.source_left_behind.as_ref(), Some(&sb.env.current_exe));
    assert_eq!(out.path, PathDecision::Edit);

    let receipt_file = receipt_path(&sb.env).unwrap();
    assert_eq!(out.receipt_path.as_ref(), Some(&receipt_file));
    let receipt = InstallReceipt::load(&receipt_file)
        .unwrap()
        .expect("receipt");
    assert_eq!(receipt.app, APP);
    assert_eq!(receipt.version, "1.2.3");
    assert_eq!(receipt.method, InstallMethod::SelfInstall);
    assert_eq!(receipt.binary_path, sb.binary());

    if sb.env.os.is_windows() {
        assert!(matches!(
            receipt.modified_path.as_slice(),
            [PathModification::WindowsUserPath { .. }]
        ));
    } else {
        let bashrc = std::fs::read_to_string(sb.home.join(".bashrc")).unwrap();
        assert_eq!(bashrc, "# existing\n. \"$HOME/.local/bin/env\"\n");
        assert!(sb.bin_dir().join("env").is_file());
    }

    // A second install is idempotent: no second rc line, same receipt shape.
    install(&sb.env, &options(), &req()).expect("reinstall");
    if !sb.env.os.is_windows() {
        let bashrc = std::fs::read_to_string(sb.home.join(".bashrc")).unwrap();
        assert_eq!(bashrc.matches("/env\"").count(), 1);
    }
    let again = InstallReceipt::load(&receipt_file).unwrap().unwrap();
    assert_eq!(again.modified_path, receipt.modified_path);

    let report = status(&sb.env);
    assert!(report.receipt.is_some());
    assert_eq!(report.bin_dir.as_ref(), Some(&sb.bin_dir()));

    let removed = uninstall(&sb.env, false).expect("uninstall");
    assert!(!sb.binary().exists());
    assert!(!receipt_file.exists());
    assert!(removed.removed.contains(&sb.binary()));
    if !sb.env.os.is_windows() {
        // The shared env file and rc line stay: other apps may rely on them.
        assert!(sb.bin_dir().join("env").is_file());
        assert!(!removed.kept.is_empty());
    }
}

#[test]
fn bootstrap_install_moves_the_download() {
    let mut sb = Sandbox::new();
    sb.env.vars.insert("CI".into(), "1".into());
    let request = InstallRequest {
        from_bootstrap: true,
        ..req()
    };
    let out = install(&sb.env, &options(), &request).expect("install");
    assert!(!sb.env.current_exe.exists(), "the download is moved");
    assert!(out.source_left_behind.is_none());
    assert!(matches!(out.path, PathDecision::Skip(_)));
    assert!(out.current_shell_hint.is_some());
    let receipt = InstallReceipt::load(&receipt_path(&sb.env).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(receipt.method, InstallMethod::Script);
    assert!(receipt.modified_path.is_empty());
    assert!(sb.downloads.read_dir().unwrap().next().is_none());
}

#[test]
fn unmanaged_install_writes_no_receipt_and_no_path_changes() {
    let sb = Sandbox::new();
    std::fs::write(sb.home.join(".profile"), "").unwrap();
    let request = InstallRequest {
        unmanaged: true,
        ..req()
    };
    let out = install(&sb.env, &options(), &request).expect("install");
    assert!(out.receipt_path.is_none());
    assert!(sb.binary().is_file());
    assert!(!receipt_path(&sb.env).unwrap().exists());
    assert_eq!(
        std::fs::read_to_string(sb.home.join(".profile")).unwrap(),
        ""
    );
    assert!(matches!(
        uninstall(&sb.env, false),
        Err(SelfInstallError::NoReceipt { .. })
    ));
}

#[test]
fn foreign_binary_is_refused_without_force() {
    let mut sb = Sandbox::new();
    sb.env.vars.insert("CI".into(), "1".into());
    std::fs::create_dir_all(sb.bin_dir()).unwrap();
    std::fs::write(sb.binary(), b"someone else's").unwrap();
    assert!(matches!(
        install(&sb.env, &options(), &req()),
        Err(SelfInstallError::ForeignBinary { .. })
    ));
    assert_eq!(std::fs::read(sb.binary()).unwrap(), b"someone else's");

    let forced = InstallRequest {
        force: true,
        ..req()
    };
    install(&sb.env, &options(), &forced).expect("forced install");
    assert_ne!(std::fs::read(sb.binary()).unwrap(), b"someone else's");
}

#[test]
fn corrupt_receipt_needs_force() {
    let mut sb = Sandbox::new();
    sb.env.vars.insert("CI".into(), "1".into());
    let receipt_file = receipt_path(&sb.env).unwrap();
    std::fs::create_dir_all(receipt_file.parent().unwrap()).unwrap();
    std::fs::write(&receipt_file, "{not json").unwrap();
    assert!(matches!(
        install(&sb.env, &options(), &req()),
        Err(SelfInstallError::Receipt(_))
    ));
    let forced = InstallRequest {
        force: true,
        ..req()
    };
    install(&sb.env, &options(), &forced).expect("forced install");
    assert!(InstallReceipt::load(&receipt_file).unwrap().is_some());
}

#[test]
fn root_is_refused_unless_allowed() {
    let mut sb = Sandbox::new();
    sb.env.vars.insert("CI".into(), "1".into());
    sb.env.is_root = true;
    assert!(matches!(
        install(&sb.env, &options(), &req()),
        Err(SelfInstallError::RunningAsRoot { .. })
    ));
    sb.env
        .vars
        .insert("DEMOAPP_INSTALL_ALLOW_SUDO".into(), "1".into());
    install(&sb.env, &options(), &req()).expect("allowed");
}

#[test]
fn uninstall_points_package_manager_installs_elsewhere() {
    let mut sb = Sandbox::new();
    sb.env.current_exe = PathBuf::from("/opt/homebrew/Cellar/demoapp/1.2.3/bin/demoapp");
    assert!(matches!(
        uninstall(&sb.env, false),
        Err(SelfInstallError::ManagedByPackageManager { .. })
    ));
    assert_eq!(status(&sb.env).method, InstallMethod::Homebrew);
}

#[test]
fn purge_removes_only_app_named_dirs() {
    let mut sb = Sandbox::new();
    sb.env.vars.insert("CI".into(), "1".into());
    install(&sb.env, &options(), &req()).expect("install");
    let config_dir = sb.env.config_root.as_ref().unwrap().join(APP);
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), "x = 1").unwrap();
    let sibling = sb.env.config_root.as_ref().unwrap().join("other-app");
    std::fs::create_dir_all(&sibling).unwrap();

    let out = uninstall(&sb.env, true).expect("uninstall --purge");
    assert!(out.purged.contains(&config_dir));
    assert!(!config_dir.exists());
    assert!(sibling.exists());
}

#[test]
fn purge_guards_refuse_dangerous_roots() {
    let mut sb = Sandbox::new();
    // A root whose `<root>/<app>` is the home directory itself.
    sb.env.app = "home".into();
    sb.env.config_root = sb.home.parent().map(Path::to_path_buf);
    assert!(matches!(
        purge_roots(&sb.env),
        Err(SelfInstallError::UnsafePurge { .. })
    ));

    let mut sb = Sandbox::new();
    sb.env.app = "..".into();
    assert!(matches!(
        purge_roots(&sb.env),
        Err(SelfInstallError::UnsafePurge { .. })
    ));
}

#[test]
fn stale_files_finds_installer_leftovers() {
    let sb = Sandbox::new();
    let bin = sb.bin_dir();
    std::fs::create_dir_all(&bin).unwrap();
    let binary = sb.binary();
    let name = binary.file_name().unwrap().to_string_lossy().into_owned();
    for leftover in [
        format!("{name}.old"),
        format!(".{name}.tmp-123"),
        format!(".{APP}-install.abc"),
        format!(".{APP}.lock"),
    ] {
        std::fs::write(bin.join(leftover), "").unwrap();
    }
    std::fs::write(&binary, "").unwrap();
    std::fs::write(bin.join("unrelated"), "").unwrap();
    assert_eq!(stale_files(&bin, APP, sb.env.os).len(), 4);
}

#[cfg(windows)]
#[test]
fn windows_user_path_edits_are_idempotent_and_reversible() {
    use cli_framework::self_install::{
        add_to_user_path, remove_from_user_path, user_path_contains,
    };
    let sb = Sandbox::new();
    let key = &sb.env.user_path_key;
    let dir = sb.bin_dir();
    assert!(!user_path_contains(key, &dir).unwrap());
    assert!(add_to_user_path(key, &dir).unwrap());
    assert!(!add_to_user_path(key, &dir).unwrap());
    let with_slash = PathBuf::from(format!("{}\\", dir.display()).to_uppercase());
    assert!(user_path_contains(key, &with_slash).unwrap());
    assert!(remove_from_user_path(key, &dir).unwrap());
    assert!(!user_path_contains(key, &dir).unwrap());
}
