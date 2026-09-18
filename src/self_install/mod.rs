//! Self-install: the `self install`, `self update`, `self rollback`,
//! `self uninstall` and `self status` commands, the install receipt, PATH
//! handling, release download and verification, the passive update notice
//! and the `install.*` doctor checks (ADR 0080).
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
//! follows the builder's built-in command namespace.
//!
//! `self update` resolves a release from the app's [`ReleaseSource`] (or a
//! mirror named by policy or `<APP>_INSTALLER_BASE_URL`), checks the
//! archive against the release's `SHA256SUMS` (and, when the app ships a
//! minisign public key, the `SHA256SUMS.minisig` signature over it), probes
//! the extracted binary's `--version`, and swaps it in. The replaced binary
//! is kept beside it as `<binary>.prev` for `self rollback`. `--from` does
//! the same from a local archive for air-gapped machines.
//!
//! Every operation takes an [`InstallEnv`]: a snapshot of the process facts
//! (home, environment variables, running executable, terminal, privilege).
//! [`InstallEnv::detect`] reads the real process; tests build one by hand
//! over temporary directories and never mutate global state.

pub mod apps_features;
mod archive;
mod commands;
mod doctor;
mod env;
mod layout;
mod method;
mod notice;
mod ops;
mod options;
mod place;
mod policy;
mod receipt;
mod release;
mod shell_path;
mod update;
#[cfg(windows)]
mod windows_path;

pub use archive::{
    check_public_key, expected_digest, extract_binary, is_safe_entry, probe_version, sha256_file,
    verify_checksum, verify_signature, ArchiveError, SIGNATURE_FILE, SUMS_FILE,
};
pub use commands::register_self_commands;
pub use doctor::install_checks;
pub use env::{InstallEnv, Os};
pub use layout::{
    binary_file_name, default_bin_dir, env_var_prefix, receipt_path, UNINSTALL_REGISTRY_KEY,
    USER_PATH_REGISTRY_KEY,
};
pub use method::{infer_method_from_path, upgrade_hint, InstallMethod};
pub use notice::{check_cache_path, notice_line, CheckCache, UpdateNotice, CHECK_CACHE_FILE};
pub use ops::{
    install, path_decision, purge_roots, status, system_bin_dir, uninstall, InstallOutcome,
    InstallRequest, PathDecision, SelfInstallError, StatusReport, UninstallOutcome,
};
pub use options::{current_target, ReleaseSource, SelfInstallOptions};
pub use place::{place_binary, stale_files, startup_cleanup, PlaceMode};
pub use policy::{SelfUpdatePolicy, KEY_BASE_URL, KEY_CHANNEL, KEY_ENABLED, KEY_MINIMUM_VERSION};
pub use receipt::{InstallReceipt, PathModification, ReceiptError, ReceiptSource};
pub use release::{
    pick_release, tag_version, Channel, Fetcher, GithubAsset, GithubRelease, ReleaseError,
    ResolvedRelease,
};
pub use shell_path::{
    apply_unix_path, fish_env_file_contents, posix_env_file_contents, rc_candidates,
    remove_fish_conf, UnixPathReport,
};
pub use update::{
    prev_path, rollback, update, RollbackOutcome, UpdateAction, UpdateOutcome, UpdateRequest,
};
#[cfg(windows)]
pub use windows_path::{add_to_user_path, remove_from_user_path, user_path_contains};
