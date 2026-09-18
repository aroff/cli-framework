//! `self update` and `self rollback` (ADR 0080, phases 2 and 3).
//!
//! Both act only on an install the receipt records for the running binary,
//! take the bin dir's lock, and keep the replaced binary as `<binary>.prev`
//! so the other can undo them. Downloads, verification and extraction all
//! happen in a temporary directory inside the bin dir: same filesystem for
//! the final rename, and never a `noexec` `/tmp`.

use super::archive::{
    extract_binary, probe_version, verify_checksum, verify_signature, ArchiveError, SIGNATURE_FILE,
    SUMS_FILE,
};
use super::env::{InstallEnv, Os};
use super::layout::{binary_file_name, env_var_prefix, receipt_path, same_path};
use super::method::{infer_method_from_path, upgrade_hint};
use super::ops::SelfInstallError;
use super::options::{check_https, current_target, ReleaseSource, SelfInstallOptions};
use super::place::{place_binary, PlaceMode};
use super::policy::{SelfUpdatePolicy, KEY_CHANNEL, KEY_ENABLED, KEY_MINIMUM_VERSION};
use super::receipt::{now_rfc3339, InstallReceipt};
use super::release::{Channel, Fetcher};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A lock older than this is left over from a crashed update.
const STALE_LOCK: Duration = Duration::from_secs(15 * 60);

/// Arguments of `self update`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateRequest {
    /// `stable`, `latest` or a version; `None` follows the receipt's channel.
    pub target: Option<String>,
    /// Report what an update would do, without downloading.
    pub check: bool,
    /// A local archive, verified against the `SHA256SUMS` beside it.
    pub from: Option<PathBuf>,
    /// Reinstall even when the version is already current.
    pub force: bool,
}

/// What `self update` found or did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAction {
    UpToDate,
    Available,
    Updated,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateOutcome {
    pub action: UpdateAction,
    pub current: String,
    pub target: String,
    /// The channel followed, when the target came from one.
    pub channel: Option<String>,
    pub binary_path: PathBuf,
    /// Where the replaced binary was kept for `self rollback`.
    pub previous: Option<PathBuf>,
    pub notes: Vec<String>,
}

/// What `self rollback` did.
#[derive(Debug, Clone, Serialize)]
pub struct RollbackOutcome {
    pub from: String,
    pub to: String,
    pub binary_path: PathBuf,
    pub notes: Vec<String>,
}

/// `<binary>.prev`: `myapp.prev`, or `myapp.exe.prev` on Windows. It does
/// not end in `.exe`, so it neither runs by name nor counts as another
/// program in the bin dir.
pub fn prev_path(binary: &Path) -> PathBuf {
    let mut name = binary.as_os_str().to_owned();
    name.push(".prev");
    PathBuf::from(name)
}

/// Load the receipt and check it describes the running binary.
pub(crate) fn managed_receipt(
    env: &InstallEnv,
) -> Result<(PathBuf, InstallReceipt), SelfInstallError> {
    let receipt_file = receipt_path(env).ok_or(SelfInstallError::NoStateDir)?;
    let receipt = InstallReceipt::load(&receipt_file)?;
    let from_path = infer_method_from_path(&env.current_exe).filter(|m| m.is_package_manager());
    let method = receipt
        .as_ref()
        .map(|r| r.method)
        .filter(|m| m.is_package_manager())
        .or(from_path);
    if let Some(m) = method {
        return Err(SelfInstallError::ManagedByPackageManager {
            app: env.app.clone(),
            method: m.as_str(),
            hint: upgrade_hint(m, &env.app).unwrap_or_default(),
        });
    }
    let Some(receipt) = receipt else {
        return Err(SelfInstallError::NotManaged {
            app: env.app.clone(),
            path: receipt_file,
            invocation: env.self_invocation.clone(),
        });
    };
    if !same_path(&receipt.binary_path, &env.current_exe, env.os) {
        return Err(SelfInstallError::ReceiptMismatch {
            recorded: receipt.binary_path.clone(),
            running: env.current_exe.clone(),
        });
    }
    Ok((receipt_file, receipt))
}

/// The channel to follow: the request, else the receipt's; an enforced
/// channel may narrow it but a request cannot widen it.
fn choose_channel(
    requested: Option<Channel>,
    recorded: &str,
    policy: &SelfUpdatePolicy,
) -> Result<Channel, SelfInstallError> {
    let channel = match requested {
        Some(c) => c,
        None => match policy.channel.as_deref() {
            Some(enforced) => Channel::parse(enforced).map_err(SelfInstallError::BadRequest)?,
            None => Channel::parse(recorded).unwrap_or(Channel::Stable),
        },
    };
    if policy.channel.as_deref() == Some("stable") {
        let widens = match &channel {
            Channel::Latest => true,
            Channel::Exact(v) => !v.pre.is_empty(),
            Channel::Stable => false,
        };
        if widens {
            return Err(SelfInstallError::PolicyRefused {
                key: KEY_CHANNEL,
                reason: "only stable releases are allowed".into(),
            });
        }
    }
    Ok(channel)
}

fn check_minimum(
    policy: &SelfUpdatePolicy,
    version: &semver::Version,
) -> Result<(), SelfInstallError> {
    match &policy.minimum_version {
        Some(min) if version < min => Err(SelfInstallError::PolicyRefused {
            key: KEY_MINIMUM_VERSION,
            reason: format!("{version} is below the minimum version {min}"),
        }),
        _ => Ok(()),
    }
}

/// The source to download from: an enforced mirror, then
/// `<APP>_INSTALLER_BASE_URL`, then the app's own declaration.
pub(crate) fn effective_source(
    env: &InstallEnv,
    opts: &SelfInstallOptions,
    policy: &SelfUpdatePolicy,
) -> Result<ReleaseSource, SelfInstallError> {
    if let Some(base_url) = &policy.base_url {
        return Ok(ReleaseSource::Http {
            base_url: base_url.clone(),
        });
    }
    let var = format!("{}_INSTALLER_BASE_URL", env_var_prefix(&env.app));
    if let Some(base_url) = env.var(&var).map(str::to_string) {
        check_https(&base_url).map_err(|e| SelfInstallError::BadRequest(format!("{var}: {e}")))?;
        return Ok(ReleaseSource::Http { base_url });
    }
    Ok(opts.source.clone())
}

/// `<APP>_GITHUB_TOKEN`, then `GITHUB_TOKEN`; only ever for a GitHub source.
pub(crate) fn github_token(env: &InstallEnv, source: &ReleaseSource) -> Option<String> {
    if !matches!(source, ReleaseSource::Github { .. }) {
        return None;
    }
    env.var(&format!("{}_GITHUB_TOKEN", env_var_prefix(&env.app)))
        .or_else(|| env.var("GITHUB_TOKEN"))
        .map(str::to_string)
}

/// The bin dir's update lock, released on drop.
pub(crate) struct UpdateLock(PathBuf);

impl UpdateLock {
    pub(crate) fn acquire(bin_dir: &Path, app: &str) -> Result<Self, SelfInstallError> {
        let path = bin_dir.join(format!(".{app}.lock"));
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let _ = std::io::Write::write_all(
                        &mut f,
                        format!("{}\n", std::process::id()).as_bytes(),
                    );
                    return Ok(Self(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|age| age > STALE_LOCK);
                    if !stale {
                        return Err(SelfInstallError::Locked { path });
                    }
                    let _ = std::fs::remove_file(&path);
                }
                Err(source) => {
                    return Err(SelfInstallError::Io {
                        action: "cannot create the update lock",
                        path,
                        source,
                    })
                }
            }
        }
        Err(SelfInstallError::Locked { path })
    }
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A temporary directory inside the bin dir, removed on drop.
pub(crate) struct WorkDir(PathBuf);

impl WorkDir {
    pub(crate) fn create(bin_dir: &Path, app: &str) -> Result<Self, SelfInstallError> {
        let path = bin_dir.join(format!(".{app}-update.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).map_err(|source| SelfInstallError::Io {
            action: "cannot create a temporary directory",
            path: path.clone(),
            source,
        })?;
        Ok(Self(path))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_text(path: &Path) -> Result<String, SelfInstallError> {
    std::fs::read_to_string(path).map_err(|source| {
        SelfInstallError::Verify(ArchiveError::Io {
            path: path.to_path_buf(),
            source,
        })
    })
}

/// Verify `archive` against the `SHA256SUMS` (and, with a public key,
/// `SHA256SUMS.minisig`) beside it, then extract the binary into `work`.
/// No network is used.
pub(crate) fn stage_local_archive(
    env: &InstallEnv,
    opts: &SelfInstallOptions,
    archive: &Path,
    work: &WorkDir,
) -> Result<(PathBuf, semver::Version), SelfInstallError> {
    let archive = if archive.is_absolute() {
        archive.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(archive))
            .unwrap_or_else(|_| archive.to_path_buf())
    };
    let dir = archive.parent().unwrap_or(Path::new("."));
    let asset = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sums = read_text(&dir.join(SUMS_FILE))?;
    if let Some(key) = &opts.public_key {
        let sig_path = dir.join(SIGNATURE_FILE);
        if !sig_path.exists() {
            return Err(ArchiveError::Unsigned(dir.display().to_string()).into());
        }
        verify_signature(sums.as_bytes(), &read_text(&sig_path)?, key)?;
    }
    verify_checksum(&archive, &asset, &sums)?;
    let binary = extract_binary(&archive, &binary_file_name(&env.app, env.os), work.path())?;
    let version = probe_version(&binary).map_err(SelfInstallError::BadRelease)?;
    Ok((binary, version))
}

/// Copy the binary about to be replaced to `<binary>.prev`. On Unix the
/// copy is not executable, so the bin dir holds one runnable `myapp`.
fn keep_previous(dest: &Path, os: Os) -> Result<PathBuf, SelfInstallError> {
    let prev = prev_path(dest);
    std::fs::copy(dest, &prev).map_err(|source| SelfInstallError::Io {
        action: "cannot keep the previous binary at",
        path: prev.clone(),
        source,
    })?;
    set_not_executable(&prev, os);
    Ok(prev)
}

#[cfg(unix)]
fn set_not_executable(path: &Path, _os: Os) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644));
}

#[cfg(not(unix))]
fn set_not_executable(_path: &Path, _os: Os) {}

fn parse_running_version(env: &InstallEnv) -> Result<semver::Version, SelfInstallError> {
    semver::Version::parse(env.version.trim_start_matches('v')).map_err(|e| {
        SelfInstallError::BadRequest(format!(
            "the running version {:?} is not semver: {e}",
            env.version
        ))
    })
}

/// Update the running binary.
pub async fn update(
    env: &InstallEnv,
    opts: &SelfInstallOptions,
    policy: &SelfUpdatePolicy,
    req: &UpdateRequest,
) -> Result<UpdateOutcome, SelfInstallError> {
    let (receipt_file, mut receipt) = managed_receipt(env)?;
    if !policy.allows_update() {
        return Err(SelfInstallError::PolicyRefused {
            key: KEY_ENABLED,
            reason: "self update is disabled".into(),
        });
    }
    let current = parse_running_version(env)?;
    let requested = req
        .target
        .as_deref()
        .map(Channel::parse)
        .transpose()
        .map_err(SelfInstallError::BadRequest)?;
    if req.from.is_some() && requested.is_some() {
        return Err(SelfInstallError::BadRequest(
            "--from installs the archive's own version; do not also name one".into(),
        ));
    }
    let channel = choose_channel(requested, &receipt.channel, policy)?;
    let bin_dir = receipt.bin_dir.clone();
    let dest = receipt.binary_path.clone();
    let outcome = |action, target: &semver::Version, previous, notes| UpdateOutcome {
        action,
        current: current.to_string(),
        target: target.to_string(),
        channel: channel.name().map(str::to_string),
        binary_path: dest.clone(),
        previous,
        notes,
    };

    // Resolve the target first, so `--check` and "already up to date" never
    // take the lock or download an archive.
    let _lock;
    let work;
    let staged;
    let target;
    if let Some(archive) = &req.from {
        _lock = UpdateLock::acquire(&bin_dir, &env.app)?;
        work = WorkDir::create(&bin_dir, &env.app)?;
        let (binary, version) = stage_local_archive(env, opts, archive, &work)?;
        check_minimum(policy, &version)?;
        if version == current && !req.force {
            return Ok(outcome(UpdateAction::UpToDate, &version, None, Vec::new()));
        }
        if req.check {
            return Ok(outcome(UpdateAction::Available, &version, None, Vec::new()));
        }
        staged = binary;
        target = version;
    } else {
        let source = effective_source(env, opts, policy)?;
        let fetcher = Fetcher::new(github_token(env, &source))?;
        let target_triple = current_target();
        let asset_for =
            |v: &semver::Version| opts.asset_name(&env.app, &v.to_string(), &target_triple);
        let release = fetcher
            .resolve(&source, &opts.tag_prefix, &asset_for, &channel)
            .await?;
        let version = release.version.clone();
        if version == current && !req.force {
            return Ok(outcome(UpdateAction::UpToDate, &version, None, Vec::new()));
        }
        if version < current && !matches!(channel, Channel::Exact(_)) {
            return Err(SelfInstallError::Downgrade {
                current: current.to_string(),
                target: version.to_string(),
                invocation: env.self_invocation.clone(),
            });
        }
        check_minimum(policy, &version)?;
        if req.check {
            return Ok(outcome(UpdateAction::Available, &version, None, Vec::new()));
        }

        _lock = UpdateLock::acquire(&bin_dir, &env.app)?;
        work = WorkDir::create(&bin_dir, &env.app)?;
        let sums_path = work.path().join(SUMS_FILE);
        fetcher
            .download(&release.sums_url, release.via_api, &sums_path)
            .await?;
        let sums = read_text(&sums_path)?;
        if let Some(key) = &opts.public_key {
            let signature = match &release.signature_url {
                Some(url) => fetcher.fetch_optional_text(url, release.via_api).await?,
                None => None,
            }
            .ok_or_else(|| ArchiveError::Unsigned(format!("release {}", release.tag)))?;
            verify_signature(sums.as_bytes(), &signature, key)?;
        }
        let archive = work.path().join(&release.asset);
        fetcher
            .download(&release.asset_url, release.via_api, &archive)
            .await?;
        verify_checksum(&archive, &release.asset, &sums)?;
        let binary = extract_binary(&archive, &binary_file_name(&env.app, env.os), work.path())?;
        let reported = probe_version(&binary).map_err(SelfInstallError::BadRelease)?;
        if reported != version {
            return Err(SelfInstallError::BadRelease(format!(
                "release {} holds a binary that reports version {reported}",
                release.tag
            )));
        }
        staged = binary;
        target = version;
    }

    let previous = keep_previous(&dest, env.os)?;
    let placed = place_binary(&staged, &dest, PlaceMode::Move, env.os).map_err(|source| {
        SelfInstallError::Place {
            path: dest.clone(),
            source,
        }
    })?;
    drop(work);

    let mut notes = Vec::new();
    if let Some(old) = &placed.parked_old {
        notes.push(format!(
            "the previous binary is still running; {} is removed on the next start",
            old.display()
        ));
    }
    receipt.previous_version = Some(current.to_string());
    receipt.version = target.to_string();
    receipt.target = current_target();
    if let Some(name) = channel.name().filter(|_| req.from.is_none()) {
        receipt.channel = name.to_string();
    }
    receipt.installed_at = now_rfc3339();
    receipt.save(&receipt_file)?;
    if let Some(key) = &receipt.apps_and_features {
        if let Err(e) = super::apps_features::set_version(key, &receipt.version) {
            notes.push(format!("Apps & Features entry not updated: {e}"));
        }
    }
    Ok(outcome(
        UpdateAction::Updated,
        &target,
        Some(previous),
        notes,
    ))
}

/// Swap the binary with the `.prev` copy the last update kept. Running it
/// twice rolls forward again.
pub fn rollback(
    env: &InstallEnv,
    policy: &SelfUpdatePolicy,
) -> Result<RollbackOutcome, SelfInstallError> {
    let (receipt_file, mut receipt) = managed_receipt(env)?;
    if !policy.allows_update() {
        return Err(SelfInstallError::PolicyRefused {
            key: KEY_ENABLED,
            reason: "self rollback is disabled".into(),
        });
    }
    let dest = receipt.binary_path.clone();
    let prev = prev_path(&dest);
    if !prev.is_file() {
        return Err(SelfInstallError::NothingToRollBack { path: prev });
    }
    let to = receipt
        .previous_version
        .clone()
        .unwrap_or_else(|| "unknown".into());
    if let Ok(v) = semver::Version::parse(&to) {
        check_minimum(policy, &v)?;
    }
    let _lock = UpdateLock::acquire(&receipt.bin_dir, &env.app)?;

    // current -> staging copy, prev -> binary, staging copy -> prev.
    let file_name = binary_file_name(&env.app, env.os);
    let hold = receipt
        .bin_dir
        .join(format!(".{file_name}.tmp-rollback-{}", std::process::id()));
    let io = |action, path: &Path| {
        let path = path.to_path_buf();
        move |source| SelfInstallError::Io {
            action,
            path,
            source,
        }
    };
    std::fs::copy(&dest, &hold).map_err(io("cannot copy the current binary to", &hold))?;
    let placed = match place_binary(&prev, &dest, PlaceMode::Move, env.os) {
        Ok(p) => p,
        Err(source) => {
            let _ = std::fs::remove_file(&hold);
            return Err(SelfInstallError::Place { path: dest, source });
        }
    };
    std::fs::rename(&hold, &prev).map_err(io("cannot keep the replaced binary at", &prev))?;
    set_not_executable(&prev, env.os);

    let mut notes = Vec::new();
    if let Some(old) = &placed.parked_old {
        notes.push(format!(
            "the replaced binary is still running; {} is removed on the next start",
            old.display()
        ));
    }
    let from = std::mem::replace(&mut receipt.version, to.clone());
    receipt.previous_version = Some(from.clone());
    receipt.installed_at = now_rfc3339();
    receipt.save(&receipt_file)?;
    if let Some(key) = &receipt.apps_and_features {
        if let Err(e) = super::apps_features::set_version(key, &receipt.version) {
            notes.push(format!("Apps & Features entry not updated: {e}"));
        }
    }
    Ok(RollbackOutcome {
        from,
        to,
        binary_path: dest,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_channel(c: &str) -> SelfUpdatePolicy {
        SelfUpdatePolicy {
            channel: Some(c.into()),
            ..Default::default()
        }
    }

    #[test]
    fn channel_defaults_to_the_receipt() {
        let p = SelfUpdatePolicy::default();
        assert_eq!(choose_channel(None, "latest", &p).unwrap(), Channel::Latest);
        assert_eq!(choose_channel(None, "bogus", &p).unwrap(), Channel::Stable);
    }

    #[test]
    fn enforced_stable_refuses_prereleases() {
        let p = policy_channel("stable");
        assert_eq!(choose_channel(None, "latest", &p).unwrap(), Channel::Stable);
        assert!(matches!(
            choose_channel(Some(Channel::Latest), "stable", &p),
            Err(SelfInstallError::PolicyRefused {
                key: KEY_CHANNEL,
                ..
            })
        ));
        let rc = Channel::parse("2.0.0-rc.1").unwrap();
        assert!(choose_channel(Some(rc), "stable", &p).is_err());
        let exact = Channel::parse("1.2.3").unwrap();
        assert!(choose_channel(Some(exact), "stable", &p).is_ok());
    }

    #[test]
    fn minimum_version_refuses_older_targets() {
        let p = SelfUpdatePolicy {
            minimum_version: Some(semver::Version::new(1, 4, 0)),
            ..Default::default()
        };
        assert!(check_minimum(&p, &semver::Version::new(1, 3, 9)).is_err());
        assert!(check_minimum(&p, &semver::Version::new(1, 4, 0)).is_ok());
    }

    #[test]
    fn prev_path_appends_a_suffix() {
        assert_eq!(
            prev_path(Path::new("/b/myapp.exe")),
            PathBuf::from("/b/myapp.exe.prev")
        );
    }

    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = UpdateLock::acquire(dir.path(), "demo").unwrap();
        assert!(matches!(
            UpdateLock::acquire(dir.path(), "demo"),
            Err(SelfInstallError::Locked { .. })
        ));
        drop(first);
        assert!(UpdateLock::acquire(dir.path(), "demo").is_ok());
    }
}
