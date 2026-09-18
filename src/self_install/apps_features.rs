//! The Windows "Apps & Features" (Add/Remove Programs) entry of a user
//! install: a key under `HKCU\...\Uninstall\<app>` whose `UninstallString`
//! runs `self uninstall`. Only per-user installs get one; `--system` installs
//! would need `HKLM` and belong to a real installer.
//!
//! Every function is a no-op returning `Ok` on other platforms, so callers
//! need no `cfg`.

use std::io;
use std::path::Path;

/// What the entry shows.
pub struct Entry<'a> {
    pub display_name: &'a str,
    pub version: &'a str,
    pub publisher: &'a str,
    pub binary: &'a Path,
    pub install_location: &'a Path,
    /// Arguments after the binary that uninstall it, e.g. `cli self uninstall`.
    pub uninstall_args: &'a str,
}

/// Create or overwrite the entry at `key` (relative to `HKEY_CURRENT_USER`).
#[cfg(windows)]
pub fn register(key: &str, entry: &Entry<'_>) -> io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let (k, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(key)?;
    let binary = entry.binary.display().to_string();
    k.set_value("DisplayName", &entry.display_name)?;
    k.set_value("DisplayVersion", &entry.version)?;
    k.set_value("Publisher", &entry.publisher)?;
    k.set_value("DisplayIcon", &binary)?;
    k.set_value(
        "UninstallString",
        &format!("\"{binary}\" {}", entry.uninstall_args),
    )?;
    k.set_value(
        "InstallLocation",
        &entry.install_location.display().to_string(),
    )?;
    k.set_value("NoModify", &1u32)?;
    k.set_value("NoRepair", &1u32)?;
    Ok(())
}

#[cfg(not(windows))]
pub fn register(_key: &str, _entry: &Entry<'_>) -> io::Result<()> {
    Ok(())
}

/// Update `DisplayVersion` after an update or rollback.
#[cfg(windows)]
pub fn set_version(key: &str, version: &str) -> io::Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(key, KEY_SET_VALUE)?
        .set_value("DisplayVersion", &version)
}

#[cfg(not(windows))]
pub fn set_version(_key: &str, _version: &str) -> io::Result<()> {
    Ok(())
}

/// Delete the entry; a missing key is not an error.
#[cfg(windows)]
pub fn remove(key: &str) -> io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    match RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(key) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(not(windows))]
pub fn remove(_key: &str) -> io::Result<()> {
    Ok(())
}

/// Whether the entry exists (always false off Windows).
#[cfg(windows)]
pub fn exists(key: &str) -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(key).is_ok()
}

#[cfg(not(windows))]
pub fn exists(_key: &str) -> bool {
    false
}
