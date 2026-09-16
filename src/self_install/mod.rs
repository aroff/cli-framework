//! Self-install: the `self install`, `self uninstall` and `self status`
//! commands, the install receipt, PATH handling and the `install.*` doctor
//! checks (ADR 0080, phase 1).
//!
//! An application opts in with one builder call:
//!
//! ```rust,no_run
//! use cli_framework::app::AppBuilder;
//! use cli_framework::self_install::SelfInstallOptions;
//!
//! let builder = AppBuilder::new()
//!     .with_version("myapp", "1.4.2")
//!     .with_self_install(SelfInstallOptions::github("aroff/myapp"));
//! ```
//!
//! The group is registered only for [`crate::Deployment::EndUser`] and
//! follows the builder's built-in command namespace. Downloading and
//! verifying a release is the installer scripts' job in phase 1; the binary
//! only places itself, so every function here works on local files.
//!
//! Every operation takes an [`InstallEnv`]: a snapshot of the process facts
//! (home, environment variables, running executable, terminal, privilege).
//! [`InstallEnv::detect`] reads the real process; tests build one by hand
//! over temporary directories and never mutate global state.

mod commands;
mod doctor;
mod env;
mod layout;
mod method;
mod ops;
mod options;
mod place;
mod receipt;
mod shell_path;
#[cfg(windows)]
mod windows_path;

pub use commands::register_self_commands;
pub use doctor::install_checks;
pub use env::{InstallEnv, Os};
pub use layout::{
    binary_file_name, default_bin_dir, env_var_prefix, receipt_path, USER_PATH_REGISTRY_KEY,
};
pub use method::{infer_method_from_path, upgrade_hint, InstallMethod};
pub use ops::{
    install, path_decision, purge_roots, status, uninstall, InstallOutcome, InstallRequest,
    PathDecision, SelfInstallError, StatusReport, UninstallOutcome,
};
pub use options::{current_target, ReleaseSource, SelfInstallOptions};
pub use place::{place_binary, stale_files, startup_cleanup, PlaceMode};
pub use receipt::{InstallReceipt, PathModification, ReceiptError, ReceiptSource};
pub use shell_path::{
    apply_unix_path, fish_env_file_contents, posix_env_file_contents, rc_candidates,
    remove_fish_conf, UnixPathReport,
};
#[cfg(windows)]
pub use windows_path::{add_to_user_path, remove_from_user_path, user_path_contains};
