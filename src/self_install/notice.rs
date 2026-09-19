//! The passive update notice (ADR 0080, phase 3): after a successful
//! command, one stderr line when a newer release exists.
//!
//! It runs after the command rather than beside it because whether it may
//! touch the network at all depends on the effective telemetry level, which
//! is only known once startup has read the stored consent. The release
//! source is asked at most once a day, with a short timeout, and the answer
//! (or the failure) is cached in `<state_root>/<app>/update-check.json`.
//!
//! Silent when: the app did not opt in; `CI` or `<APP>_NO_UPDATE_CHECK` is
//! set; stderr is not a terminal; telemetry is off; policy disables
//! updates; or the command was the `self` group, `completion` or
//! `mcp serve`.

use super::env::InstallEnv;
use super::layout::env_var_prefix;
use super::method::{infer_method_from_path, upgrade_command};
use super::ops::SelfInstallError;
use super::options::SelfInstallOptions;
use super::policy::SelfUpdatePolicy;
use super::release::{Channel, Fetcher};
use super::update::{effective_source, github_token, managed_receipt};
use serde::{Deserialize, Serialize};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The cache file's name inside `<state_root>/<app>`.
pub const CHECK_CACHE_FILE: &str = "update-check.json";
/// How long an answer, or a failure, stands.
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The notice never holds a finished command up for longer than this.
const CHECK_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckCache {
    /// Unix seconds of the last attempt, successful or not.
    pub checked_at: u64,
    /// The newest version found; `None` when the attempt failed.
    pub latest: Option<String>,
}

/// Everything the notice needs, captured at build time.
pub struct UpdateNotice {
    app: &'static str,
    version: &'static str,
    options: Arc<SelfInstallOptions>,
    self_invocation: String,
    /// argv prefixes (after the binary) the notice never follows.
    exempt: Vec<Vec<String>>,
}

impl UpdateNotice {
    pub(crate) fn new(
        app: &'static str,
        version: &'static str,
        options: Arc<SelfInstallOptions>,
        namespace: &[String],
    ) -> Self {
        let with = |leaf: &str| {
            let mut v = namespace.to_vec();
            v.push(leaf.to_string());
            v
        };
        let mut self_invocation = vec![app.to_string()];
        self_invocation.extend(with("self"));
        Self {
            app,
            version,
            options,
            self_invocation: self_invocation.join(" "),
            exempt: vec![
                with("self"),
                with("completion"),
                vec!["mcp".into(), "serve".into()],
            ],
        }
    }

    fn exempt(&self, argv: &[String]) -> bool {
        let words: Vec<&str> = argv
            .iter()
            .skip(1)
            .map(String::as_str)
            .filter(|a| !a.starts_with('-'))
            .collect();
        self.exempt.iter().any(|prefix| {
            prefix.len() <= words.len() && prefix.iter().zip(&words).all(|(p, w)| p == w)
        })
    }

    /// Run after a successful command. Never fails and never prints
    /// anything but the notice itself.
    pub(crate) async fn run(
        &self,
        argv: &[String],
        telemetry_off: bool,
        policy: &SelfUpdatePolicy,
    ) {
        if telemetry_off || !policy.allows_update() || self.exempt(argv) {
            return;
        }
        if !std::io::stderr().is_terminal() {
            return;
        }
        let Ok(mut env) = InstallEnv::detect(self.app, self.version) else {
            return;
        };
        env.self_invocation = self.self_invocation.clone();
        if let Some(line) = notice_line(&env, &self.options, policy).await {
            crate::app::diagnostic_reporter::DiagnosticReporter::write_plain(&line);
        }
    }
}

/// The notice for this environment, if one is due. Split from
/// [`UpdateNotice::run`] so tests drive it without a terminal.
pub async fn notice_line(
    env: &InstallEnv,
    opts: &SelfInstallOptions,
    policy: &SelfUpdatePolicy,
) -> Option<String> {
    if env.var("CI").is_some()
        || env
            .var(&format!("{}_NO_UPDATE_CHECK", env_var_prefix(&env.app)))
            .is_some()
    {
        return None;
    }
    // Only a binary that can act on the notice gets one: a self-managed
    // install gets `self update`, a package-manager install its own command.
    let (channel, action) = match managed_receipt(env) {
        Ok((_, receipt)) => (
            Channel::parse(&receipt.channel).unwrap_or(Channel::Stable),
            format!("run `{} update`", env.self_invocation),
        ),
        // The upgrade command alone: the hint's "(remove: ...)" is not something to run.
        Err(SelfInstallError::ManagedByPackageManager { .. }) => {
            let command = infer_method_from_path(&env.current_exe)
                .and_then(|m| upgrade_command(m, &env.app))?;
            (Channel::Stable, format!("run `{command}`"))
        }
        Err(_) => return None,
    };
    let channel = match policy.channel.as_deref() {
        Some("stable") => Channel::Stable,
        _ => channel,
    };
    let current = semver::Version::parse(env.version.trim_start_matches('v')).ok()?;
    let cache_path = env
        .state_root
        .as_ref()?
        .join(&env.app)
        .join(CHECK_CACHE_FILE);
    let latest = match fresh_cache(&cache_path) {
        Some(cache) => cache.latest,
        None => {
            let latest = check(env, opts, policy, &channel).await;
            write_cache(
                &cache_path,
                &CheckCache {
                    checked_at: unix_now(),
                    latest: latest.clone(),
                },
            );
            latest
        }
    };
    let latest = semver::Version::parse(&latest?).ok()?;
    (latest > current).then(|| {
        format!(
            "{} {latest} is available (you have {current}); {action}",
            env.app
        )
    })
}

async fn check(
    env: &InstallEnv,
    opts: &SelfInstallOptions,
    policy: &SelfUpdatePolicy,
    channel: &Channel,
) -> Option<String> {
    let source = effective_source(env, opts, policy).ok()?;
    let client = super::release::client_with_timeout(CHECK_TIMEOUT).ok()?;
    let fetcher = Fetcher::with_client(client, github_token(env, &source));
    let lookup = fetcher.newest_version(&source, &opts.tag_prefix, channel);
    match tokio::time::timeout(CHECK_TIMEOUT, lookup).await {
        Ok(Ok(v)) => Some(v.to_string()),
        _ => None,
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The cached answer while it is under a day old. A timestamp in the
/// future (a clock set back) counts as stale.
fn fresh_cache(path: &Path) -> Option<CheckCache> {
    let text = std::fs::read_to_string(path).ok()?;
    let cache: CheckCache = serde_json::from_str(&text).ok()?;
    let age = unix_now().checked_sub(cache.checked_at)?;
    (age < CHECK_INTERVAL.as_secs()).then_some(cache)
}

fn write_cache(path: &Path, cache: &CheckCache) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(cache) {
        let _ = std::fs::write(path, text);
    }
}

/// Where the cache lives for `env`, for tests and `self status`.
pub fn check_cache_path(env: &InstallEnv) -> Option<PathBuf> {
    Some(
        env.state_root
            .as_ref()?
            .join(&env.app)
            .join(CHECK_CACHE_FILE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice() -> UpdateNotice {
        UpdateNotice::new(
            "demo",
            "1.0.0",
            Arc::new(SelfInstallOptions::github("o/demo")),
            &["cli".to_string()],
        )
    }

    fn argv(words: &[&str]) -> Vec<String> {
        std::iter::once("demo")
            .chain(words.iter().copied())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn exempt_commands_are_recognised() {
        let n = notice();
        assert!(n.exempt(&argv(&["cli", "self", "update"])));
        assert!(n.exempt(&argv(&["--verbose", "cli", "completion", "bash"])));
        assert!(n.exempt(&argv(&["mcp", "serve"])));
        assert!(!n.exempt(&argv(&["run", "self"])));
        assert!(!n.exempt(&argv(&["cli"])));
    }

    #[test]
    fn cache_expires_after_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHECK_CACHE_FILE);
        let fresh = CheckCache {
            checked_at: unix_now() - 60,
            latest: Some("1.2.0".into()),
        };
        write_cache(&path, &fresh);
        assert_eq!(fresh_cache(&path), Some(fresh));
        write_cache(
            &path,
            &CheckCache {
                checked_at: unix_now() - CHECK_INTERVAL.as_secs() - 1,
                latest: None,
            },
        );
        assert_eq!(fresh_cache(&path), None);
        write_cache(
            &path,
            &CheckCache {
                checked_at: unix_now() + 3600,
                latest: None,
            },
        );
        assert_eq!(fresh_cache(&path), None);
    }
}
