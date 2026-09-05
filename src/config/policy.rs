//! [`Policy`]: the document a managed-configuration server serves for one
//! profile (spec 021, ADR 0072).
//!
//! `Policy` is pure data — no networking. It is deliberately available
//! whenever the `config` feature is enabled (not gated behind
//! `config-managed`), so [`crate::config::resolution`] can be unit tested
//! against hand-built `Policy` values without pulling in HTTP/auth
//! machinery. The networked *fetcher* (`PolicyClient`) lives under
//! `config-managed` — see `crate::config::managed`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A policy document: two trees of configuration values (`enforced`,
/// `recommended`) plus enough metadata to know how long a cached copy stays
/// valid and what to do once it doesn't.
///
/// Both trees are flat JSON objects keyed by the same dotted leaf-path
/// coordinate system as [`crate::config::manifest::ConfigManifest::iter_leaves`]
/// — e.g. `{"network.proxy_url": "http://..."}`, never a nested object
/// mirroring the manifest's section structure. There is no third tree (spec
/// 021: "There is no third tree").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub contract_version: u32,
    pub app: String,
    pub profile: String,
    pub policy_version: u64,
    pub max_cache_age_secs: u64,
    pub stale_action: StaleAction,
    #[serde(default)]
    pub enforced: Map<String, Value>,
    #[serde(default)]
    pub recommended: Map<String, Value>,
}

/// What a client does with a cached [`Policy`] once it has aged past
/// `max_cache_age_secs` and the server cannot be reached to revalidate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleAction {
    /// Proceed on the stale policy, with a loud diagnostic.
    Warn,
    /// Refuse to start rather than run on rules that might be out of date.
    Refuse,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Policy {
        Policy {
            contract_version: 1,
            app: "myapp".to_string(),
            profile: "developers".to_string(),
            policy_version: 3,
            max_cache_age_secs: 3600,
            stale_action: StaleAction::Warn,
            enforced: Map::new(),
            recommended: Map::new(),
        }
    }

    #[test]
    fn round_trips_through_json() {
        let mut policy = sample();
        policy
            .enforced
            .insert("network.proxy_url".to_string(), json!("http://proxy"));
        policy
            .recommended
            .insert("telemetry.level".to_string(), json!("usage"));

        let value = serde_json::to_value(&policy).unwrap();
        let back: Policy = serde_json::from_value(value).unwrap();
        assert_eq!(back, policy);
    }

    #[test]
    fn stale_action_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(StaleAction::Refuse).unwrap(),
            json!("refuse")
        );
        assert_eq!(
            serde_json::to_value(StaleAction::Warn).unwrap(),
            json!("warn")
        );
    }

    #[test]
    fn missing_trees_default_to_empty() {
        let value = json!({
            "contract_version": 1,
            "app": "myapp",
            "profile": "developers",
            "policy_version": 1,
            "max_cache_age_secs": 60,
            "stale_action": "warn",
        });
        let policy: Policy = serde_json::from_value(value).unwrap();
        assert!(policy.enforced.is_empty());
        assert!(policy.recommended.is_empty());
    }
}
