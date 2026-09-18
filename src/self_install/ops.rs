//! The three operations behind `self install`, `self uninstall` and
//! `self status`. Pure over an [`InstallEnv`]: no global state is read here,
//! and nothing is printed.

use super::archive::ArchiveError;
use super::env::InstallEnv;
use super::layout::{binary_file_name, default_bin_dir, env_var_prefix, receipt_path, same_path};
use super::method::{infer_method_from_path, upgrade_hint, InstallMethod};
use super::options::{current_target, SelfInstallOptions};
use super::place::{place_binary, stale_files, PlaceMode, PlaceResult};
use super::receipt::{
    now_rfc3339, InstallReceipt, PathModification, ReceiptError, ReceiptSource,
    RECEIPT_SCHEMA_VERSION,
};
use super::release::ReleaseError;
use super::update::{prev_path, stage_local_archive, UpdateLock, WorkDir};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Flags of `self install`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallRequest {
    pub bin_dir: Option<PathBuf>,
    pub no_modify_path: bool,
    pub unmanaged: bool,
    pub force: bool,
    pub from_bootstrap: bool,
    /// Install into the machine-wide bin dir (`/usr/local/bin`, or
    /// `%ProgramFiles%\<app>\bin`). PATH, completions and Apps & Features
    /// are left alone.
    pub system: bool,
    /// Install from a local release archive instead of the running binary.
    pub from: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum SelfInstallError {
    #[error(
        "refusing to install as root: the binary and PATH changes would land in root's home. \
         Run as your own user, or set {allow_var}=1 if root's home is really the target"
    )]
    RunningAsRoot { allow_var: String },
    #[error("cannot determine a bin dir (no home directory); pass --bin-dir")]
    NoBinDir,
    #[error("cannot determine where the install receipt lives (no local data directory)")]
    NoStateDir,
    #[error(
        "{path} already exists and was not installed by `{invocation} install`; \
         pass --force to replace it"
    )]
    ForeignBinary { path: PathBuf, invocation: String },
    #[error(transparent)]
    Receipt(#[from] ReceiptError),
    #[error("cannot place the binary at {path}: {source}")]
    Place {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{app} was installed by {method}; use: {hint}")]
    ManagedByPackageManager {
        app: String,
        method: &'static str,
        hint: String,
    },
    #[error(
        "no install receipt at {path}; {app} was not installed with `{invocation} install`, \
         so there is nothing recorded to remove. Delete the binary yourself: {binary}"
    )]
    NoReceipt {
        app: String,
        path: PathBuf,
        invocation: String,
        binary: PathBuf,
    },
    #[error("refusing to purge {path}: {reason}")]
    UnsafePurge { path: PathBuf, reason: &'static str },
    #[error("{action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{dir} is not writable by this user; run: {command}")]
    NeedsElevation { dir: PathBuf, command: String },
    #[error(
        "no install receipt at {path}; {app} was not installed with `{invocation} install`, \
         so it cannot update itself. Update it the way it was installed"
    )]
    NotManaged {
        app: String,
        path: PathBuf,
        invocation: String,
    },
    #[error(
        "the install receipt records {recorded}, but this is {running}; \
         run the installed binary instead"
    )]
    ReceiptMismatch { recorded: PathBuf, running: PathBuf },
    #[error("refused by your organisation's policy ({key}): {reason}")]
    PolicyRefused { key: &'static str, reason: String },
    #[error(transparent)]
    Release(#[from] ReleaseError),
    #[error(transparent)]
    Verify(#[from] ArchiveError),
    #[error("the release is unusable: {0}")]
    BadRelease(String),
    #[error(
        "{target} is older than the running {current}; \
         name the version to downgrade: `{invocation} update {target}`"
    )]
    Downgrade {
        current: String,
        target: String,
        invocation: String,
    },
    #[error("nothing to roll back to: {path} does not exist")]
    NothingToRollBack { path: PathBuf },
    #[error("{0}")]
    BadRequest(String),
    #[error("another update is running (lock {path}); if none is, delete the lock file")]
    Locked { path: PathBuf },
}

/// Whether PATH gets edited, and why not when it does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", content = "reason", rename_all = "snake_case")]
pub enum PathDecision {
    Edit,
    AlreadyOnPath,
    Skip(String),
}

/// The ADR's rule: never edit under `--no-modify-path`, `--unmanaged`, `CI`,
/// or when stdout is not a terminal.
pub fn path_decision(env: &InstallEnv, req: &InstallRequest, bin_dir: &Path) -> PathDecision {
    if req.system {
        if on_path(env, bin_dir) {
            return PathDecision::AlreadyOnPath;
        }
        return PathDecision::Skip("--system: the system PATH is the administrator's".into());
    }
    if req.unmanaged {
        return PathDecision::Skip("--unmanaged".into());
    }
    if req.no_modify_path {
        return PathDecision::Skip("--no-modify-path".into());
    }
    if env.var("CI").is_some() {
        return PathDecision::Skip("CI is set".into());
    }
    if !env.stdout_is_terminal {
        return PathDecision::Skip("stdout is not a terminal".into());
    }
    if on_path(env, bin_dir) {
        return PathDecision::AlreadyOnPath;
    }
    PathDecision::Edit
}

fn on_path(env: &InstallEnv, dir: &Path) -> bool {
    env.path_entries()
        .iter()
        .any(|entry| same_path(entry, dir, env.os))
}

/// What `self install` did.
#[derive(Debug, Clone, Serialize)]
pub struct InstallOutcome {
    /// The version installed: the running one, or the `--from` archive's.
    pub version: String,
    pub binary_path: PathBuf,
    pub bin_dir: PathBuf,
    pub replaced_existing: bool,
    pub already_in_place: bool,
    /// Receipt location; `None` for `--unmanaged`.
    pub receipt_path: Option<PathBuf>,
    pub path: PathDecision,
    pub path_modifications: Vec<PathModification>,
    /// Command for the current shell when the bin dir is not on its PATH.
    pub current_shell_hint: Option<String>,
    pub completions: Vec<PathBuf>,
    pub notes: Vec<String>,
    /// The copy that was run, when it was copied rather than moved.
    pub source_left_behind: Option<PathBuf>,
}

/// Install the running binary.
pub fn install(
    env: &InstallEnv,
    opts: &SelfInstallOptions,
    req: &InstallRequest,
) -> Result<InstallOutcome, SelfInstallError> {
    let prefix = env_var_prefix(&env.app);
    let allow_var = format!("{prefix}_INSTALL_ALLOW_SUDO");
    if env.is_root && !req.system && env.var(&allow_var).is_none() {
        return Err(SelfInstallError::RunningAsRoot { allow_var });
    }
    if req.system && (req.bin_dir.is_some() || req.unmanaged) {
        return Err(SelfInstallError::BadRequest(
            "--system chooses the bin dir and records a receipt; \
             drop --bin-dir and --unmanaged"
                .into(),
        ));
    }

    let bin_dir = match &req.bin_dir {
        Some(dir) if dir.is_absolute() => dir.clone(),
        Some(dir) => std::env::current_dir()
            .map(|cwd| cwd.join(dir))
            .unwrap_or_else(|_| dir.clone()),
        None if req.system => system_bin_dir(env),
        None => default_bin_dir(env).ok_or(SelfInstallError::NoBinDir)?,
    };
    if req.system {
        probe_writable(env, req, &bin_dir)?;
    }
    let dest = bin_dir.join(binary_file_name(&env.app, env.os));

    let receipt_file = if req.unmanaged {
        None
    } else {
        Some(receipt_path(env).ok_or(SelfInstallError::NoStateDir)?)
    };
    let previous = match &receipt_file {
        Some(p) => match InstallReceipt::load(p) {
            Ok(r) => r,
            // --force also recovers from a corrupt receipt: it is rewritten.
            Err(_) if req.force => None,
            Err(e) => return Err(e.into()),
        },
        None => None,
    };

    let ours = previous
        .as_ref()
        .is_some_and(|r| same_path(&r.binary_path, &dest, env.os));
    let running_is_dest = dest.exists() && same_path(&env.current_exe, &dest, env.os);
    if dest.exists() && !running_is_dest && !ours && !req.force {
        return Err(SelfInstallError::ForeignBinary {
            path: dest,
            invocation: env.self_invocation.clone(),
        });
    }

    // `--from`: verify and unpack the archive into the bin dir first, so the
    // final placement is a same-filesystem rename.
    let (_lock, work, staged) = match &req.from {
        Some(archive) => {
            std::fs::create_dir_all(&bin_dir).map_err(|source| SelfInstallError::Io {
                action: "cannot create",
                path: bin_dir.clone(),
                source,
            })?;
            let lock = UpdateLock::acquire(&bin_dir, &env.app)?;
            let work = WorkDir::create(&bin_dir, &env.app)?;
            let (binary, version) = stage_local_archive(env, opts, archive, &work)?;
            (Some(lock), Some(work), Some((binary, version.to_string())))
        }
        None => (None, None, None),
    };
    let (src, version) = match &staged {
        Some((binary, version)) => (binary.clone(), version.clone()),
        None => (env.current_exe.clone(), env.version.clone()),
    };
    let mode = if req.from_bootstrap || staged.is_some() {
        PlaceMode::Move
    } else {
        PlaceMode::Copy
    };
    let placed: PlaceResult =
        place_binary(&src, &dest, mode, env.os).map_err(|source| SelfInstallError::Place {
            path: dest.clone(),
            source,
        })?;
    drop(work);

    let mut notes = Vec::new();
    if let Some(old) = &placed.parked_old {
        notes.push(format!(
            "the previous binary is still running; {} is removed on the next start",
            old.display()
        ));
    }
    let source_left_behind =
        (!placed.already_in_place && mode == PlaceMode::Copy).then(|| env.current_exe.clone());

    let decision = path_decision(env, req, &bin_dir);
    let mut modifications = Vec::new();
    let mut current_shell_hint = None;
    match decision {
        PathDecision::Edit => {
            let (mods, hint, mut path_notes) = edit_path(env, &bin_dir)?;
            modifications = mods;
            current_shell_hint = hint;
            notes.append(&mut path_notes);
        }
        PathDecision::Skip(_) if !on_path(env, &bin_dir) => {
            current_shell_hint = Some(manual_path_hint(env, &bin_dir));
        }
        _ => {}
    }

    if req.unmanaged {
        return Ok(InstallOutcome {
            version,
            binary_path: dest,
            bin_dir,
            replaced_existing: placed.replaced_existing,
            already_in_place: placed.already_in_place,
            receipt_path: None,
            path: decision,
            path_modifications: modifications,
            current_shell_hint,
            completions: Vec::new(),
            notes,
            source_left_behind,
        });
    }

    let completions = if opts.completions && !req.system {
        install_completions(env, &dest, &mut notes)
    } else {
        Vec::new()
    };

    // A reinstall keeps what earlier installs recorded, so uninstall still
    // reverses all of it.
    let mut all_mods = previous
        .as_ref()
        .map(|r| r.modified_path.clone())
        .unwrap_or_default();
    for m in &modifications {
        if !all_mods.contains(m) {
            all_mods.push(m.clone());
        }
    }
    let mut all_completions = previous
        .as_ref()
        .map(|r| r.completions.clone())
        .unwrap_or_default();
    for c in &completions {
        if !all_completions.contains(c) {
            all_completions.push(c.clone());
        }
    }

    let apps_and_features = if env.os.is_windows() && opts.apps_and_features && !req.system {
        register_apps_and_features(env, opts, &version, &dest, &bin_dir, &mut notes)
    } else {
        None
    };
    // An update's `.prev` survives a reinstall, and so does what it records.
    let previous_version = previous
        .as_ref()
        .and_then(|r| r.previous_version.clone())
        .filter(|_| prev_path(&dest).is_file());

    let receipt = InstallReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        app: env.app.clone(),
        version: version.clone(),
        target: current_target(),
        channel: previous
            .as_ref()
            .map(|r| r.channel.clone())
            .unwrap_or_else(|| "stable".into()),
        bin_dir: bin_dir.clone(),
        binary_path: dest.clone(),
        method: if req.from_bootstrap {
            InstallMethod::Script
        } else {
            InstallMethod::SelfInstall
        },
        modified_path: all_mods,
        completions: all_completions,
        source: ReceiptSource::from(opts),
        installed_at: now_rfc3339(),
        previous_version,
        system: req.system,
        apps_and_features,
    };
    let receipt_file = receipt_file.expect("managed install has a receipt path");
    receipt.save(&receipt_file)?;

    Ok(InstallOutcome {
        version,
        binary_path: dest,
        bin_dir,
        replaced_existing: placed.replaced_existing,
        already_in_place: placed.already_in_place,
        receipt_path: Some(receipt_file),
        path: decision,
        path_modifications: modifications,
        current_shell_hint,
        completions,
        notes,
        source_left_behind,
    })
}

/// The machine-wide bin dir: `/usr/local/bin`, or
/// `%ProgramFiles%\<app>\bin` on Windows.
pub fn system_bin_dir(env: &InstallEnv) -> PathBuf {
    if env.os.is_windows() {
        let program_files = env.var("ProgramFiles").unwrap_or(r"C:\Program Files");
        PathBuf::from(program_files).join(&env.app).join("bin")
    } else {
        PathBuf::from("/usr/local/bin")
    }
}

/// `--system` needs write access to the bin dir. Check before anything is
/// placed, and name the elevated command rather than elevating.
fn probe_writable(
    env: &InstallEnv,
    req: &InstallRequest,
    bin_dir: &Path,
) -> Result<(), SelfInstallError> {
    let probe = bin_dir.join(format!(".{}.write-probe-{}", env.app, std::process::id()));
    let result = std::fs::create_dir_all(bin_dir).and_then(|_| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .map(drop)
    });
    let _ = std::fs::remove_file(&probe);
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(SelfInstallError::NeedsElevation {
                dir: bin_dir.to_path_buf(),
                command: elevated_command(env, req),
            })
        }
        Err(source) => Err(SelfInstallError::Io {
            action: "cannot write to",
            path: bin_dir.to_path_buf(),
            source,
        }),
    }
}

fn elevated_command(env: &InstallEnv, req: &InstallRequest) -> String {
    let group = env
        .self_invocation
        .split_once(' ')
        .map(|(_, rest)| rest)
        .unwrap_or("self");
    let mut args = format!("{group} install --system");
    if let Some(from) = &req.from {
        args.push_str(&format!(" --from \"{}\"", from.display()));
    }
    let exe = env.current_exe.display();
    if env.os.is_windows() {
        format!(
            "Start-Process -Verb RunAs -FilePath \"{exe}\" -ArgumentList '{args}' \
             (in PowerShell)"
        )
    } else {
        format!("sudo \"{exe}\" {args}")
    }
}

fn register_apps_and_features(
    env: &InstallEnv,
    opts: &SelfInstallOptions,
    version: &str,
    binary: &Path,
    bin_dir: &Path,
    notes: &mut Vec<String>,
) -> Option<String> {
    let key = format!("{}\\{}", env.uninstall_key_root, env.app);
    let group = env
        .self_invocation
        .split_once(' ')
        .map(|(_, rest)| rest)
        .unwrap_or("self");
    let entry = super::apps_features::Entry {
        display_name: &env.app,
        version,
        publisher: &opts.publisher_name(&env.app),
        binary,
        install_location: bin_dir,
        uninstall_args: &format!("{group} uninstall"),
    };
    match super::apps_features::register(&key, &entry) {
        Ok(()) => Some(key),
        Err(e) => {
            notes.push(format!("no Apps & Features entry: {e}"));
            None
        }
    }
}

type PathEdit = (Vec<PathModification>, Option<String>, Vec<String>);

#[cfg(not(windows))]
fn edit_path(env: &InstallEnv, bin_dir: &Path) -> Result<PathEdit, SelfInstallError> {
    let report = super::shell_path::apply_unix_path(env, bin_dir).map_err(|source| {
        SelfInstallError::Io {
            action: "cannot update shell startup files for",
            path: bin_dir.to_path_buf(),
            source,
        }
    })?;
    let hint = (!report.current_shell_hint.is_empty()).then_some(report.current_shell_hint);
    Ok((report.modifications, hint, report.notes))
}

#[cfg(windows)]
fn edit_path(env: &InstallEnv, bin_dir: &Path) -> Result<PathEdit, SelfInstallError> {
    let added =
        super::windows_path::add_to_user_path(&env.user_path_key, bin_dir).map_err(|source| {
            SelfInstallError::Io {
                action: "cannot add to the user Path",
                path: bin_dir.to_path_buf(),
                source,
            }
        })?;
    let mods = if added {
        vec![PathModification::WindowsUserPath {
            key: env.user_path_key.clone(),
            entry: bin_dir.to_string_lossy().into_owned(),
        }]
    } else {
        Vec::new()
    };
    Ok((mods, Some(manual_path_hint(env, bin_dir)), Vec::new()))
}

fn manual_path_hint(env: &InstallEnv, bin_dir: &Path) -> String {
    if env.os.is_windows() {
        format!("$env:Path = \"{};$env:Path\"", bin_dir.display())
    } else {
        let expr = super::shell_path::shell_dir_expr(env.home.as_deref(), bin_dir)
            .unwrap_or_else(|| bin_dir.display().to_string());
        format!("export PATH=\"{expr}:$PATH\"")
    }
}

/// Write bash and fish completion files by running the installed binary's
/// own `completion` command. Failures become notes: completions are a
/// convenience and never fail an install.
fn install_completions(env: &InstallEnv, binary: &Path, notes: &mut Vec<String>) -> Vec<PathBuf> {
    let Some(argv) = &env.completion_command else {
        notes.push("completions are enabled but the completion command is disabled".into());
        return Vec::new();
    };
    if env.os.is_windows() {
        return Vec::new();
    }
    let Some(home) = env.home.as_deref() else {
        return Vec::new();
    };
    let data_home = env
        .var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("share"));
    let config_home = env
        .var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let mut targets = vec![(
        "bash",
        data_home
            .join("bash-completion")
            .join("completions")
            .join(&env.app),
    )];
    if config_home.join("fish").is_dir() {
        targets.push((
            "fish",
            config_home
                .join("fish")
                .join("completions")
                .join(format!("{}.fish", env.app)),
        ));
    }
    let mut written = Vec::new();
    for (shell, path) in targets {
        let output = std::process::Command::new(binary)
            .args(argv)
            .arg(shell)
            .output();
        match output {
            Ok(o) if o.status.success() && !o.stdout.is_empty() => {
                let res = path
                    .parent()
                    .map(std::fs::create_dir_all)
                    .unwrap_or(Ok(()))
                    .and_then(|_| std::fs::write(&path, &o.stdout));
                match res {
                    Ok(()) => written.push(path),
                    Err(e) => notes.push(format!("{shell} completions not written: {e}")),
                }
            }
            Ok(o) => notes.push(format!(
                "{shell} completions not written: completion command exited with {}",
                o.status
            )),
            Err(e) => notes.push(format!("{shell} completions not written: {e}")),
        }
    }
    written
}

/// What `self uninstall` did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UninstallOutcome {
    pub removed: Vec<PathBuf>,
    /// Shared PATH changes left in place, and why.
    pub kept: Vec<String>,
    pub purged: Vec<PathBuf>,
    /// On Windows the running binary is deleted by a helper after exit.
    pub deletion_deferred: bool,
}

/// The directories `--purge` removes, after the guards.
pub fn purge_roots(env: &InstallEnv) -> Result<Vec<PathBuf>, SelfInstallError> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for root in [&env.config_root, &env.state_root, &env.data_root]
        .into_iter()
        .flatten()
    {
        let dir = root.join(&env.app);
        if !roots.iter().any(|r| same_path(r, &dir, env.os)) {
            roots.push(dir);
        }
    }
    for dir in &roots {
        guard_purge(env, dir)?;
    }
    Ok(roots)
}

fn guard_purge(env: &InstallEnv, dir: &Path) -> Result<(), SelfInstallError> {
    let unsafe_purge = |reason| SelfInstallError::UnsafePurge {
        path: dir.to_path_buf(),
        reason,
    };
    if env.app.is_empty() || env.app.contains(['/', '\\']) || env.app == "." || env.app == ".." {
        return Err(unsafe_purge("the application name is not a plain name"));
    }
    if dir.parent().is_none_or(|p| p.parent().is_none()) {
        return Err(unsafe_purge("it is at or directly under a filesystem root"));
    }
    if let Some(home) = &env.home {
        if same_path(dir, home, env.os) || home.starts_with(dir) {
            return Err(unsafe_purge("it is the home directory or contains it"));
        }
    }
    if dir.file_name().map(|n| n.to_string_lossy()) != Some(env.app.as_str().into()) {
        return Err(unsafe_purge("it is not named after the application"));
    }
    Ok(())
}

/// Reverse what the receipt records. With `purge`, also remove the config
/// and data roots named by [`purge_roots`]. Keychain entries are never
/// touched.
pub fn uninstall(env: &InstallEnv, purge: bool) -> Result<UninstallOutcome, SelfInstallError> {
    let receipt_file = receipt_path(env).ok_or(SelfInstallError::NoStateDir)?;
    let Some(receipt) = InstallReceipt::load(&receipt_file)? else {
        let method = infer_method_from_path(&env.current_exe);
        if let Some(m) = method.filter(|m| m.is_package_manager()) {
            return Err(SelfInstallError::ManagedByPackageManager {
                app: env.app.clone(),
                method: m.as_str(),
                hint: upgrade_hint(m, &env.app).unwrap_or_default(),
            });
        }
        return Err(SelfInstallError::NoReceipt {
            app: env.app.clone(),
            path: receipt_file,
            invocation: env.self_invocation.clone(),
            binary: env.current_exe.clone(),
        });
    };
    let roots = if purge { purge_roots(env)? } else { Vec::new() };

    let mut out = UninstallOutcome::default();
    for file in &receipt.completions {
        if remove_if_exists(file)? {
            out.removed.push(file.clone());
        }
    }
    let binary_name = binary_file_name(&env.app, env.os);
    for m in &receipt.modified_path {
        match m {
            PathModification::FishConf { file } => {
                if remove_if_exists(file)? {
                    out.removed.push(file.clone());
                }
            }
            PathModification::RcLine { file, line } => out.kept.push(format!(
                "{}: `{line}` (shared with other apps using this bin dir)",
                file.display()
            )),
            PathModification::EnvFile { file } => out.kept.push(format!(
                "{} (shared with other apps using this bin dir)",
                file.display()
            )),
            PathModification::WindowsUserPath { key, entry } => {
                let dir = PathBuf::from(entry);
                if other_executables(&dir, &binary_name) {
                    out.kept.push(format!(
                        "user Path entry {entry} (other programs still live there)"
                    ));
                } else {
                    remove_windows_path(key, &dir, &mut out)?;
                }
            }
        }
    }

    let prev = prev_path(&receipt.binary_path);
    if remove_if_exists(&prev)? {
        out.removed.push(prev);
    }
    if let Some(key) = &receipt.apps_and_features {
        match super::apps_features::remove(key) {
            Ok(()) => out.removed.push(PathBuf::from(format!("HKCU\\{key}"))),
            Err(e) => out
                .kept
                .push(format!("Apps & Features entry HKCU\\{key} ({e})")),
        }
    }
    for stale in stale_files(&receipt.bin_dir, &env.app, env.os) {
        if stale.is_file() && remove_if_exists(&stale).unwrap_or(false) {
            out.removed.push(stale);
        }
    }

    remove_if_exists(&receipt_file)?;
    out.removed.push(receipt_file.clone());
    if let Some(dir) = receipt_file.parent() {
        let _ = std::fs::remove_dir(dir);
    }

    for dir in roots {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|source| SelfInstallError::Io {
                action: "cannot remove",
                path: dir.clone(),
                source,
            })?;
            out.purged.push(dir);
        }
    }

    // Last: on Windows deleting the running binary hands off to a helper
    // process that waits for this one to exit.
    let binary = &receipt.binary_path;
    if binary.exists() {
        if same_path(binary, &env.current_exe, env.os) {
            self_replace::self_delete().map_err(|source| SelfInstallError::Io {
                action: "cannot remove",
                path: binary.clone(),
                source,
            })?;
            out.deletion_deferred = env.os.is_windows();
        } else {
            remove_if_exists(binary)?;
        }
        out.removed.push(binary.clone());
    }
    Ok(out)
}

#[cfg(windows)]
fn remove_windows_path(
    key: &str,
    dir: &Path,
    out: &mut UninstallOutcome,
) -> Result<(), SelfInstallError> {
    if super::windows_path::remove_from_user_path(key, dir).map_err(|source| {
        SelfInstallError::Io {
            action: "cannot edit the user Path for",
            path: dir.to_path_buf(),
            source,
        }
    })? {
        out.removed.push(PathBuf::from(format!(
            "HKCU\\{key}\\Path: {}",
            dir.display()
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_windows_path(
    _key: &str,
    dir: &Path,
    out: &mut UninstallOutcome,
) -> Result<(), SelfInstallError> {
    out.kept.push(format!(
        "user Path entry {} (recorded on Windows; nothing to do here)",
        dir.display()
    ));
    Ok(())
}

/// Whether `dir` holds an executable other than ours (by extension on
/// Windows, where this is asked).
fn other_executables(dir: &Path, ours: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_lowercase();
        name != ours.to_lowercase() && name.ends_with(".exe") && !name.contains(".exe.old")
    })
}

fn remove_if_exists(path: &Path) -> Result<bool, SelfInstallError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(SelfInstallError::Io {
            action: "cannot remove",
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// The shape of `self status --json`. Every key always serializes.
#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub app: String,
    pub version: String,
    pub target: String,
    pub running_binary: PathBuf,
    pub method: InstallMethod,
    pub receipt_path: Option<PathBuf>,
    pub receipt: Option<InstallReceipt>,
    pub receipt_error: Option<String>,
    /// The receipt's bin dir, or the default one when there is no receipt.
    pub bin_dir: Option<PathBuf>,
    pub bin_dir_on_path: bool,
    /// The first `app` found on PATH.
    pub resolved_on_path: Option<PathBuf>,
    /// Every copy of `app` on PATH, in PATH order.
    pub copies_on_path: Vec<PathBuf>,
    pub stale_files: Vec<PathBuf>,
    pub upgrade_hint: Option<String>,
}

/// Describe how this binary is installed.
pub fn status(env: &InstallEnv) -> StatusReport {
    let receipt_file = receipt_path(env);
    let (receipt, receipt_error) = match receipt_file.as_deref().map(InstallReceipt::load) {
        Some(Ok(r)) => (r, None),
        Some(Err(e)) => (None, Some(e.to_string())),
        None => (None, None),
    };
    let method = match (&receipt, infer_method_from_path(&env.current_exe)) {
        (Some(r), _) => r.method,
        (None, Some(m)) => m,
        (None, None) => InstallMethod::Unknown,
    };
    let bin_dir = receipt
        .as_ref()
        .map(|r| r.bin_dir.clone())
        .or_else(|| default_bin_dir(env));
    let copies = copies_on_path(env);
    StatusReport {
        app: env.app.clone(),
        version: env.version.clone(),
        target: current_target(),
        running_binary: env.current_exe.clone(),
        method,
        receipt_path: receipt_file,
        bin_dir_on_path: bin_dir.as_deref().is_some_and(|d| on_path(env, d)),
        stale_files: bin_dir
            .as_deref()
            .map(|d| stale_files(d, &env.app, env.os))
            .unwrap_or_default(),
        bin_dir,
        resolved_on_path: copies.first().cloned(),
        copies_on_path: copies,
        upgrade_hint: upgrade_hint(method, &env.app),
        receipt,
        receipt_error,
    }
}

fn copies_on_path(env: &InstallEnv) -> Vec<PathBuf> {
    let name = binary_file_name(&env.app, env.os);
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in env.path_entries() {
        let candidate = dir.join(&name);
        if candidate.is_file() && !found.iter().any(|f| same_path(f, &candidate, env.os)) {
            found.push(candidate);
        }
    }
    found
}
