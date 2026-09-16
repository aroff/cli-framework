//! How a copy of the binary got onto the machine.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallMethod {
    /// `myapp self install` run by a person.
    SelfInstall,
    /// `self install --from-bootstrap`, run by an installer script.
    Script,
    /// `--unmanaged`: no receipt is written, so this only appears in status.
    Unmanaged,
    Homebrew,
    Scoop,
    Winget,
    Cargo,
    Unknown,
}

impl InstallMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            InstallMethod::SelfInstall => "self-install",
            InstallMethod::Script => "script",
            InstallMethod::Unmanaged => "unmanaged",
            InstallMethod::Homebrew => "homebrew",
            InstallMethod::Scoop => "scoop",
            InstallMethod::Winget => "winget",
            InstallMethod::Cargo => "cargo",
            InstallMethod::Unknown => "unknown",
        }
    }

    pub fn is_package_manager(self) -> bool {
        matches!(
            self,
            InstallMethod::Homebrew
                | InstallMethod::Scoop
                | InstallMethod::Winget
                | InstallMethod::Cargo
        )
    }
}

/// Recognise a package manager from where the executable lives. Checks the
/// path as given and its canonical form, because Homebrew's
/// `/opt/homebrew/bin/myapp` is a symlink into the Cellar.
pub fn infer_method_from_path(path: &Path) -> Option<InstallMethod> {
    let canonical = std::fs::canonicalize(path).ok();
    [Some(path.to_path_buf()), canonical]
        .into_iter()
        .flatten()
        .find_map(|p| classify(&p.to_string_lossy()))
}

fn classify(raw: &str) -> Option<InstallMethod> {
    let p = raw.replace('\\', "/").to_lowercase();
    if p.contains("/cellar/")
        || p.starts_with("/opt/homebrew/")
        || p.starts_with("/home/linuxbrew/.linuxbrew/")
    {
        Some(InstallMethod::Homebrew)
    } else if p.contains("/scoop/apps/") || p.contains("/scoop/shims/") {
        Some(InstallMethod::Scoop)
    } else if p.contains("/winget/") {
        Some(InstallMethod::Winget)
    } else if p.contains("/.cargo/bin/") {
        Some(InstallMethod::Cargo)
    } else {
        None
    }
}

/// The command that upgrades or removes a package-manager install.
pub fn upgrade_hint(method: InstallMethod, app: &str) -> Option<String> {
    match method {
        InstallMethod::Homebrew => Some(format!(
            "brew upgrade {app}  (remove: brew uninstall {app})"
        )),
        InstallMethod::Scoop => Some(format!(
            "scoop update {app}  (remove: scoop uninstall {app})"
        )),
        InstallMethod::Winget => Some(format!(
            "winget upgrade {app}  (remove: winget uninstall {app})"
        )),
        InstallMethod::Cargo => Some(format!(
            "cargo install {app} --force  (remove: cargo uninstall {app})"
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_package_manager_layouts() {
        let cases = [
            (
                "/opt/homebrew/Cellar/myapp/1.0/bin/myapp",
                InstallMethod::Homebrew,
            ),
            (
                "/usr/local/Cellar/myapp/1.0/bin/myapp",
                InstallMethod::Homebrew,
            ),
            (
                "/home/linuxbrew/.linuxbrew/bin/myapp",
                InstallMethod::Homebrew,
            ),
            (
                r"C:\Users\a\scoop\apps\myapp\current\myapp.exe",
                InstallMethod::Scoop,
            ),
            (
                r"C:\Users\a\AppData\Local\Microsoft\WinGet\Packages\x\myapp.exe",
                InstallMethod::Winget,
            ),
            ("/home/a/.cargo/bin/myapp", InstallMethod::Cargo),
        ];
        for (path, expected) in cases {
            assert_eq!(classify(path), Some(expected), "{path}");
        }
        assert_eq!(classify("/home/a/.local/bin/myapp"), None);
    }
}
