//! Inheritance flattening, exercised through the public
//! `cli_framework::config::service` surface — the house-convention external
//! target complementing `src/config/service/inherit.rs`'s inline
//! pure-function suite.

use cli_framework::config::service::{flatten, resolve_chain, InheritanceError, StoredPolicy};
use cli_framework::config::StaleAction;
use serde_json::json;
use std::collections::HashMap;

fn policy(profile: &str, parent: Option<&str>, enforced: serde_json::Value) -> StoredPolicy {
    StoredPolicy {
        app: "myapp".to_string(),
        profile: profile.to_string(),
        enforced: enforced.as_object().cloned().unwrap_or_default(),
        recommended: Default::default(),
        parent_profile: parent.map(str::to_string),
        max_cache_age_secs: 3600,
        stale_action: StaleAction::Warn,
        version: 1,
    }
}

fn by_profile(policies: &[StoredPolicy]) -> HashMap<&str, &StoredPolicy> {
    policies.iter().map(|p| (p.profile.as_str(), p)).collect()
}

#[test]
fn a_child_field_overrides_the_parents_value_for_the_same_key() {
    let policies = vec![
        policy("base", None, json!({"theme": "light"})),
        policy("dark-mode", Some("base"), json!({"theme": "dark"})),
    ];
    let map = by_profile(&policies);
    let chain = resolve_chain(&map, "dark-mode").unwrap();
    let (enforced, _) = flatten(&chain);
    assert_eq!(enforced.get("theme"), Some(&json!("dark")));
}

#[test]
fn a_parent_only_field_still_appears_in_the_flattened_result() {
    let policies = vec![
        policy("base", None, json!({"telemetry": true})),
        policy("child", Some("base"), json!({"theme": "dark"})),
    ];
    let map = by_profile(&policies);
    let chain = resolve_chain(&map, "child").unwrap();
    let (enforced, _) = flatten(&chain);
    assert_eq!(enforced.get("telemetry"), Some(&json!(true)));
    assert_eq!(enforced.get("theme"), Some(&json!("dark")));
}

#[test]
fn a_two_level_chain_flattens_grandparent_parent_and_child_together() {
    let policies = vec![
        policy("grandparent", None, json!({"a": "gp"})),
        policy("parent", Some("grandparent"), json!({"b": "p"})),
        policy("child", Some("parent"), json!({"c": "c"})),
    ];
    let map = by_profile(&policies);
    let chain = resolve_chain(&map, "child").unwrap();
    let (enforced, _) = flatten(&chain);
    assert_eq!(enforced.get("a"), Some(&json!("gp")));
    assert_eq!(enforced.get("b"), Some(&json!("p")));
    assert_eq!(enforced.get("c"), Some(&json!("c")));
}

#[test]
fn a_two_node_cycle_is_rejected() {
    let policies = vec![
        policy("a", Some("b"), json!({})),
        policy("b", Some("a"), json!({})),
    ];
    let map = by_profile(&policies);
    assert!(matches!(
        resolve_chain(&map, "a").unwrap_err(),
        InheritanceError::Cycle { .. }
    ));
}

#[test]
fn a_longer_cycle_is_also_rejected_not_just_the_two_node_case() {
    let policies = vec![
        policy("a", Some("b"), json!({})),
        policy("b", Some("c"), json!({})),
        policy("c", Some("a"), json!({})),
    ];
    let map = by_profile(&policies);
    assert!(matches!(
        resolve_chain(&map, "a").unwrap_err(),
        InheritanceError::Cycle { .. }
    ));
}
