//! Where things go: bin dir, binary name, receipt path.

use super::env::{InstallEnv, Os};
use std::path::{Path, PathBuf};

/// The `HKEY_CURRENT_USER` subkey holding the per-user `Path`.
pub const USER_PATH_REGISTRY_KEY: &str = "Environment";

/// The `HKEY_CURRENT_USER` subkey whose children are Apps & Features entries.
pub const UNINSTALL_REGISTRY_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

/// The receipt's file name inside `<state_root>/<app>`.
pub const RECEIPT_FILE: &str = "install-receipt.json";

/// `myapp` becomes `MYAPP`, `my-app.x` becomes `MY_APP_X`. The same rule the
/// telemetry variables use, so an app's variables share one prefix.
pub fn env_var_prefix(app: &str) -> String {
    app.to_ascii_uppercase().replace(['-', '.'], "_")
}

/// `myapp` on Unix, `myapp.exe` on Windows.
pub fn binary_file_name(app: &str, os: Os) -> String {
    if os.is_windows() {
        format!("{app}.exe")
    } else {
        app.to_string()
    }
}

/// The per-user bin dir: `$XDG_BIN_HOME` or `~/.local/bin` on Unix,
/// `%USERPROFILE%\.local\bin` on Windows. `None` only when there is no home.
pub fn default_bin_dir(env: &InstallEnv) -> Option<PathBuf> {
    if !env.os.is_windows() {
        if let Some(xdg) = env.var("XDG_BIN_HOME") {
            let p = PathBuf::from(xdg);
            if p.is_absolute() {
                return Some(p);
            }
        }
    }
    env.home.as_ref().map(|h| h.join(".local").join("bin"))
}

/// `<state_root>/<app>/install-receipt.json`.
pub fn receipt_path(env: &InstallEnv) -> Option<PathBuf> {
    env.state_root
        .as_ref()
        .map(|r| r.join(&env.app).join(RECEIPT_FILE))
}

/// Compare two paths as the filesystem would: canonical when both exist,
/// and case-insensitive on Windows.
pub(crate) fn same_path(a: &Path, b: &Path, os: Os) -> bool {
    let ca = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    if os.is_windows() {
        normalize_windows(&ca) == normalize_windows(&cb)
    } else {
        ca == cb
    }
}

fn normalize_windows(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    s.trim_end_matches('\\').to_string()
}
