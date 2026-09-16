//! Putting the executable into the bin dir, and cleaning up after it.

use super::env::Os;
use super::layout::{binary_file_name, same_path};
use std::io;
use std::path::{Path, PathBuf};

/// Whether the source file survives placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceMode {
    /// A person ran a downloaded binary: copy it and say it can be deleted.
    Copy,
    /// An installer script ran it from its temp dir: move it, so the download
    /// leaves nothing behind.
    Move,
}

/// What placement did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaceResult {
    /// The source already was the destination; nothing moved.
    pub already_in_place: bool,
    /// A previous binary at the destination was replaced.
    pub replaced_existing: bool,
    /// Windows could not delete the replaced binary because it is running;
    /// it was renamed here and is removed at the next start.
    pub parked_old: Option<PathBuf>,
}

/// Place `src` at `dest` without ever leaving a half-written executable.
///
/// The new file is first written to a temporary name in the destination
/// directory (same filesystem, never `/tmp`, which may be `noexec`). Unix then
/// renames over the destination, which is atomic and safe while the old
/// binary runs. Windows cannot replace a running executable, so it renames
/// the old one to `<name>.old` first and restores it if the final rename
/// fails.
pub fn place_binary(src: &Path, dest: &Path, mode: PlaceMode, os: Os) -> io::Result<PlaceResult> {
    if dest.exists() && same_path(src, dest, os) {
        return Ok(PlaceResult {
            already_in_place: true,
            ..Default::default()
        });
    }
    let dir = dest
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    std::fs::create_dir_all(dir)?;
    let file_name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{file_name}.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);

    let staged = match mode {
        PlaceMode::Move => std::fs::rename(src, &tmp).or_else(|_| {
            // Different filesystem: copy, then drop the source.
            std::fs::copy(src, &tmp)?;
            let _ = std::fs::remove_file(src);
            Ok::<(), io::Error>(())
        }),
        PlaceMode::Copy => std::fs::copy(src, &tmp).map(|_| ()),
    };
    if let Err(e) = staged {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    make_executable(&tmp)?;

    let mut result = PlaceResult {
        replaced_existing: dest.exists(),
        ..Default::default()
    };

    if os.is_windows() && dest.exists() {
        let old = park_old(dest)?;
        if let Err(e) = std::fs::rename(&tmp, dest) {
            let _ = std::fs::rename(&old, dest);
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        if std::fs::remove_file(&old).is_err() {
            result.parked_old = Some(old);
        }
    } else if let Err(e) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    clear_download_marks(dest);
    Ok(result)
}

/// Rename `dest` to `dest.old`, or `dest.old.<pid>` when an older `.old` is
/// itself still running.
fn park_old(dest: &Path) -> io::Result<PathBuf> {
    let base = format!("{}.old", dest.display());
    let old = PathBuf::from(&base);
    let _ = std::fs::remove_file(&old);
    let old = if old.exists() {
        PathBuf::from(format!("{base}.{}", std::process::id()))
    } else {
        old
    };
    std::fs::rename(dest, &old)?;
    Ok(old)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Remove the marks a browser or `curl` download leaves, so Gatekeeper and
/// SmartScreen do not block a binary the installer already verified.
/// Best-effort: absence of the mark is the normal case.
fn clear_download_marks(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        if let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) {
            // SAFETY: both pointers are valid NUL-terminated strings for the
            // duration of the call; failure (usually ENOATTR) is ignored.
            unsafe {
                libc::removexattr(c_path.as_ptr(), c"com.apple.quarantine".as_ptr(), 0);
            }
        }
    }
    #[cfg(windows)]
    {
        let stream = format!("{}:Zone.Identifier", path.display());
        let _ = std::fs::remove_file(stream);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
    }
}

/// Delete `<exe>.old` files left beside the running executable by a Windows
/// replacement. Called once by the builder; never fails and does nothing on
/// other platforms.
pub fn startup_cleanup() {
    #[cfg(windows)]
    {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let (Some(dir), Some(name)) = (exe.parent(), exe.file_name()) else {
            return;
        };
        let prefix = format!("{}.old", name.to_string_lossy()).to_lowercase();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .starts_with(&prefix)
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Leftovers in the bin dir: `.old` executables, installer-script temp dirs,
/// staging files and the update lock.
pub fn stale_files(bin_dir: &Path, app: &str, os: Os) -> Vec<PathBuf> {
    let binary = binary_file_name(app, os);
    let old_prefix = format!("{binary}.old");
    let script_tmp = format!(".{app}-install.");
    let staging = format!(".{binary}.tmp-");
    let lock = format!(".{app}.lock");
    let Ok(entries) = std::fs::read_dir(bin_dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(&old_prefix)
                || name.starts_with(&script_tmp)
                || name.starts_with(&staging)
                || name == lock
        })
        .map(|e| e.path())
        .collect();
    found.sort();
    found
}
