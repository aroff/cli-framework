//! A snapshot of the process facts every self-install operation reads.

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::PathBuf;

/// The operating system family, as far as installation cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Macos,
    Windows,
    /// Any other Unix; treated like Linux.
    OtherUnix,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::OtherUnix
        }
    }

    pub fn is_windows(self) -> bool {
        self == Os::Windows
    }
}

/// Everything an operation needs to know about the machine and process.
///
/// Built by [`InstallEnv::detect`] in production. Tests construct it with
/// temporary directories so nothing touches the real home, PATH or registry.
#[derive(Debug, Clone)]
pub struct InstallEnv {
    pub app: String,
    pub version: String,
    pub os: Os,
    pub home: Option<PathBuf>,
    /// Environment variables, captured once.
    pub vars: BTreeMap<String, String>,
    pub current_exe: PathBuf,
    pub stdout_is_terminal: bool,
    /// Effective uid 0 on Unix. Always false on Windows.
    pub is_root: bool,
    /// Machine-local state root (`dirs::data_local_dir`); the receipt lives
    /// in `<state_root>/<app>`.
    pub state_root: Option<PathBuf>,
    /// Config root (`dirs::config_dir`), touched only by `--purge`.
    pub config_root: Option<PathBuf>,
    /// Roaming data root (`dirs::data_dir`), touched only by `--purge`.
    pub data_root: Option<PathBuf>,
    /// Registry key under `HKEY_CURRENT_USER` whose `Path` value is edited on
    /// Windows. `Environment` in production; a scratch key in tests.
    pub user_path_key: String,
    /// The argv prefix that reaches the built-in `completion` command, when
    /// the application kept it, e.g. `["cli", "completion"]`.
    pub completion_command: Option<Vec<String>>,
    /// How a person invokes the self group, e.g. `myapp cli self`, used in
    /// messages.
    pub self_invocation: String,
}

impl InstallEnv {
    /// Read the real process.
    pub fn detect(app: &str, version: &str) -> std::io::Result<Self> {
        let current_exe = std::env::current_exe()?;
        Ok(Self {
            app: app.to_string(),
            version: version.to_string(),
            os: Os::current(),
            home: dirs::home_dir(),
            vars: std::env::vars().collect(),
            current_exe,
            stdout_is_terminal: std::io::stdout().is_terminal(),
            is_root: effective_uid_is_root(),
            state_root: dirs::data_local_dir(),
            config_root: dirs::config_dir(),
            data_root: dirs::data_dir(),
            user_path_key: crate::self_install::layout::USER_PATH_REGISTRY_KEY.to_string(),
            completion_command: None,
            self_invocation: format!("{app} self"),
        })
    }

    /// A variable's value, treating empty as unset.
    pub fn var(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// The `PATH` entries, in order.
    pub fn path_entries(&self) -> Vec<PathBuf> {
        let raw = self
            .vars
            .iter()
            .find(|(k, _)| {
                if self.os.is_windows() {
                    k.eq_ignore_ascii_case("PATH")
                } else {
                    k.as_str() == "PATH"
                }
            })
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        let sep = if self.os.is_windows() { ';' } else { ':' };
        raw.split(sep)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect()
    }
}

#[cfg(unix)]
fn effective_uid_is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn effective_uid_is_root() -> bool {
    false
}
