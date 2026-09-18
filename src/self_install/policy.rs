//! The `self_update.*` keys an organisation enforces through managed
//! configuration (ADR 0080, "Enterprise controls").
//!
//! Only the `enforced` tree counts: a recommendation cannot stop a person
//! from updating their own binary. Every refusal names the key that caused
//! it, so the person knows whom to ask.

use crate::config::Policy;
use serde_json::Value;

pub const KEY_ENABLED: &str = "self_update.enabled";
pub const KEY_CHANNEL: &str = "self_update.channel";
pub const KEY_BASE_URL: &str = "self_update.base_url";
pub const KEY_MINIMUM_VERSION: &str = "self_update.minimum_version";

/// The enforced `self_update.*` values. `Default` is "no policy".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelfUpdatePolicy {
    /// `false` disables `self update`, `self rollback` and the notice.
    pub enabled: Option<bool>,
    /// `stable` or `latest`: the only channel updates may follow.
    pub channel: Option<String>,
    /// An HTTPS mirror that replaces the app's release source.
    pub base_url: Option<String>,
    /// No update or rollback may land below this version.
    pub minimum_version: Option<semver::Version>,
    /// Keys present with a value of the wrong type or shape. They are
    /// reported, and the safe reading applies: a malformed `enabled` counts
    /// as `false`.
    pub malformed: Vec<String>,
}

impl SelfUpdatePolicy {
    /// Read the keys from a policy document's `enforced` tree.
    pub fn from_policy(policy: &Policy) -> Self {
        let get = |key: &str| policy.enforced.get(key);
        let mut out = Self::default();
        match get(KEY_ENABLED) {
            None => {}
            Some(Value::Bool(b)) => out.enabled = Some(*b),
            Some(_) => {
                out.enabled = Some(false);
                out.malformed.push(KEY_ENABLED.into());
            }
        }
        match get(KEY_CHANNEL) {
            None => {}
            Some(Value::String(s)) if s == "stable" || s == "latest" => {
                out.channel = Some(s.clone())
            }
            Some(_) => out.malformed.push(KEY_CHANNEL.into()),
        }
        match get(KEY_BASE_URL) {
            None => {}
            Some(Value::String(s)) if super::options::check_https(s).is_ok() => {
                out.base_url = Some(s.clone())
            }
            Some(_) => out.malformed.push(KEY_BASE_URL.into()),
        }
        match get(KEY_MINIMUM_VERSION) {
            None => {}
            Some(Value::String(s)) => match semver::Version::parse(s.trim_start_matches('v')) {
                Ok(v) => out.minimum_version = Some(v),
                Err(_) => out.malformed.push(KEY_MINIMUM_VERSION.into()),
            },
            Some(_) => out.malformed.push(KEY_MINIMUM_VERSION.into()),
        }
        out
    }

    /// Whether updates are allowed at all.
    pub fn allows_update(&self) -> bool {
        self.enabled != Some(false)
    }

    /// The app's cached managed policy, read without any network request.
    /// No policy client (or the `config-managed` feature off) means no
    /// policy.
    pub(crate) fn from_context(ctx: &dyn crate::app::context::AppContext) -> Self {
        #[cfg(feature = "config-managed")]
        if let Some(client) = ctx.opt_policy_client() {
            if let Ok(Some(policy)) = client.cached_policy() {
                return Self::from_policy(&policy);
            }
        }
        let _ = ctx;
        Self::default()
    }
}
