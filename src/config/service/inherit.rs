//! Inheritance resolution (spec 022, "Inheritance"): a stored policy may
//! declare a single `parent_profile`; the chain is resolved by deep-merging
//! parent trees beneath the child's before serving, so the wire document
//! (`crate::config::Policy`) has zero representation of inheritance.
//!
//! Cycles are rejected both at startup (`super::validate::validate_all`,
//! over every stored policy in one pass) and again here, defensively, at
//! read time — bounded traversal that refuses rather than infinite-loops on
//! a cycle that somehow got past startup validation (spec 022: "refuse
//! rather than infinite-loop").

use super::error::InheritanceError;
use super::types::StoredPolicy;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Walk `start`'s `parent_profile` chain within `policies_by_profile` (all
/// stored policies for one application, keyed by profile name), returning
/// the chain **child-first, root-last** — `chain[0]` is always the policy
/// for `start` itself.
///
/// Bounded by `policies_by_profile.len()`: a well-formed chain can never be
/// longer than the number of distinct profiles that exist, so exceeding
/// that bound is itself proof of a cycle, independent of the `seen`-set
/// check below (belt and braces — either one alone would already catch
/// every real cycle; both exist because the bound also protects against a
/// caller passing a `policies_by_profile` that doesn't actually contain
/// `start`'s own chain, which the `seen` check alone would not bound).
pub fn resolve_chain<'a>(
    policies_by_profile: &HashMap<&str, &'a StoredPolicy>,
    start: &str,
) -> Result<Vec<&'a StoredPolicy>, InheritanceError> {
    let mut chain: Vec<&'a StoredPolicy> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut current = start.to_string();
    let bound = policies_by_profile.len().max(1);

    loop {
        let Some(policy) = policies_by_profile.get(current.as_str()) else {
            return if chain.is_empty() {
                Err(InheritanceError::ProfileNotFound { profile: current })
            } else {
                Err(InheritanceError::MissingParent {
                    child: chain.last().unwrap().profile.clone(),
                    parent: current,
                })
            };
        };

        if !seen.insert(current.clone()) {
            return Err(InheritanceError::Cycle { profile: current });
        }
        chain.push(*policy);

        if chain.len() > bound {
            return Err(InheritanceError::Cycle {
                profile: start.to_string(),
            });
        }

        match &policy.parent_profile {
            Some(parent) => current = parent.clone(),
            None => break,
        }
    }

    Ok(chain)
}

/// Combine every profile's stored `version` in a resolved chain (as
/// returned by [`resolve_chain`]) into a single `u64` token that changes
/// whenever **any** profile in the chain changes, not just the leaf's own.
///
/// This exists because a served, flattened [`crate::config::Policy`] is the
/// result of the *entire* chain, but each [`StoredPolicy`] only carries its
/// own version. Using the leaf's version alone as a cache key / ETag (the
/// bug this function fixes) means a parent-only edit is invisible to both:
/// the cache key doesn't change, so a stale flattened value keeps being
/// served, and the ETag doesn't change either, so a client's conditional
/// `If-None-Match` can get a stale `304` on top of that.
///
/// A chain of exactly one profile (no inheritance — by far the common case)
/// degenerates to that profile's own stored version, unchanged, so nothing
/// observable differs for a policy with no parent.
///
/// For a chain of two or more, every profile's name and version are folded
/// into one hash, in the fixed child-first order [`resolve_chain`] returns.
/// This is a practical combiner, not a formally collision-free one — like
/// any hash-based ETag, two distinct chain states could in principle hash
/// to the same token — but the collision probability with a 64-bit SipHash
/// output is low enough that this is the same trade-off ETags make
/// elsewhere in practice.
pub fn combined_chain_version(chain: &[&StoredPolicy]) -> u64 {
    if let [only] = chain {
        return only.version;
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for policy in chain {
        policy.profile.hash(&mut hasher);
        policy.version.hash(&mut hasher);
    }
    hasher.finish()
}

/// Deep-merge a child-first chain (as returned by [`resolve_chain`]) into
/// one `(enforced, recommended)` pair: root applied first, each descendant
/// applied over it, so the child wins on a conflicting key and a
/// parent-only field still appears untouched.
pub fn flatten(chain: &[&StoredPolicy]) -> (Map<String, Value>, Map<String, Value>) {
    let mut enforced = Map::new();
    let mut recommended = Map::new();
    for policy in chain.iter().rev() {
        for (k, v) in &policy.enforced {
            enforced.insert(k.clone(), v.clone());
        }
        for (k, v) in &policy.recommended {
            recommended.insert(k.clone(), v.clone());
        }
    }
    (enforced, recommended)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StaleAction;
    use serde_json::json;

    fn policy(
        profile: &str,
        parent: Option<&str>,
        enforced: Value,
        recommended: Value,
    ) -> StoredPolicy {
        StoredPolicy {
            app: "myapp".to_string(),
            profile: profile.to_string(),
            enforced: enforced.as_object().cloned().unwrap_or_default(),
            recommended: recommended.as_object().cloned().unwrap_or_default(),
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
    fn no_parent_resolves_to_a_chain_of_one() {
        let policies = vec![policy("base", None, json!({}), json!({}))];
        let map = by_profile(&policies);
        let chain = resolve_chain(&map, "base").unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].profile, "base");
    }

    #[test]
    fn two_level_chain_flattens_correctly() {
        let policies = vec![
            policy("base", None, json!({"a": 1, "b": 1}), json!({})),
            policy("child", Some("base"), json!({"b": 2}), json!({})),
        ];
        let map = by_profile(&policies);
        let chain = resolve_chain(&map, "child").unwrap();
        assert_eq!(
            chain.iter().map(|p| p.profile.as_str()).collect::<Vec<_>>(),
            vec!["child", "base"]
        );

        let (enforced, _) = flatten(&chain);
        // Child wins on the conflicting key...
        assert_eq!(enforced.get("b"), Some(&json!(2)));
        // ...and a parent-only field still appears.
        assert_eq!(enforced.get("a"), Some(&json!(1)));
    }

    #[test]
    fn child_wins_is_not_vacuous_reversing_merge_direction_changes_the_result() {
        let policies = vec![
            policy("base", None, json!({"x": "parent"}), json!({})),
            policy("child", Some("base"), json!({"x": "child"}), json!({})),
        ];
        let map = by_profile(&policies);
        let chain = resolve_chain(&map, "child").unwrap();
        let (enforced, _) = flatten(&chain);
        assert_eq!(enforced.get("x"), Some(&json!("child")));

        // The anti-vacuity check itself: applying the *reversed* chain
        // (as if merge direction were flipped) must produce the opposite,
        // wrong answer -- proving the assertion above is actually sensitive
        // to merge direction rather than trivially true.
        let mut reversed = chain.clone();
        reversed.reverse();
        let (enforced_reversed, _) = flatten(&reversed);
        assert_eq!(enforced_reversed.get("x"), Some(&json!("parent")));
    }

    #[test]
    fn recommended_tree_flattens_independently_of_enforced() {
        let policies = vec![
            policy("base", None, json!({}), json!({"r": "parent"})),
            policy("child", Some("base"), json!({}), json!({"r": "child"})),
        ];
        let map = by_profile(&policies);
        let chain = resolve_chain(&map, "child").unwrap();
        let (_, recommended) = flatten(&chain);
        assert_eq!(recommended.get("r"), Some(&json!("child")));
    }

    #[test]
    fn direct_cycle_is_rejected() {
        let policies = vec![
            policy("a", Some("b"), json!({}), json!({})),
            policy("b", Some("a"), json!({}), json!({})),
        ];
        let map = by_profile(&policies);
        let err = resolve_chain(&map, "a").unwrap_err();
        assert!(matches!(err, InheritanceError::Cycle { .. }), "got {err:?}");
    }

    #[test]
    fn self_referential_cycle_is_rejected() {
        let policies = vec![policy("a", Some("a"), json!({}), json!({}))];
        let map = by_profile(&policies);
        let err = resolve_chain(&map, "a").unwrap_err();
        assert!(matches!(err, InheritanceError::Cycle { .. }), "got {err:?}");
    }

    #[test]
    fn missing_start_profile_is_profile_not_found_not_missing_parent() {
        let policies: Vec<StoredPolicy> = vec![];
        let map = by_profile(&policies);
        let err = resolve_chain(&map, "ghost").unwrap_err();
        assert!(
            matches!(err, InheritanceError::ProfileNotFound { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn missing_ancestor_is_missing_parent_not_profile_not_found() {
        let policies = vec![policy("child", Some("ghost-parent"), json!({}), json!({}))];
        let map = by_profile(&policies);
        let err = resolve_chain(&map, "child").unwrap_err();
        assert!(
            matches!(err, InheritanceError::MissingParent { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn combined_chain_version_of_a_single_node_chain_is_that_nodes_own_version() {
        let policies = vec![policy("base", None, json!({}), json!({}))];
        let map = by_profile(&policies);
        let chain = resolve_chain(&map, "base").unwrap();
        assert_eq!(combined_chain_version(&chain), chain[0].version);
    }

    #[test]
    fn combined_chain_version_changes_when_only_the_ancestors_version_changes() {
        let policies_v1 = vec![
            policy("base", None, json!({"a": 1}), json!({})),
            policy("child", Some("base"), json!({}), json!({})),
        ];
        let map_v1 = by_profile(&policies_v1);
        let chain_v1 = resolve_chain(&map_v1, "child").unwrap();
        let token_v1 = combined_chain_version(&chain_v1);

        // Bump only the parent's stored version; the child's own row (and
        // therefore `chain[0].version`, the old buggy cache key/ETag basis)
        // is completely unchanged.
        let mut base_v2 = policy("base", None, json!({"a": 2}), json!({}));
        base_v2.version = 2;
        let policies_v2 = vec![base_v2, policy("child", Some("base"), json!({}), json!({}))];
        let map_v2 = by_profile(&policies_v2);
        let chain_v2 = resolve_chain(&map_v2, "child").unwrap();
        let token_v2 = combined_chain_version(&chain_v2);

        assert_eq!(
            chain_v1[0].version, chain_v2[0].version,
            "sanity check: the leaf's own version is unchanged between the two chains"
        );
        assert_ne!(
            token_v1, token_v2,
            "combined_chain_version must change when only an ancestor's version changes"
        );
    }

    #[test]
    fn combined_chain_version_is_deterministic_for_the_same_chain_contents() {
        let policies = vec![
            policy("base", None, json!({}), json!({})),
            policy("child", Some("base"), json!({}), json!({})),
        ];
        let map = by_profile(&policies);
        let chain_a = resolve_chain(&map, "child").unwrap();
        let chain_b = resolve_chain(&map, "child").unwrap();
        assert_eq!(
            combined_chain_version(&chain_a),
            combined_chain_version(&chain_b)
        );
    }

    #[test]
    fn three_level_chain_flattens_across_all_levels() {
        let policies = vec![
            policy(
                "grandparent",
                None,
                json!({"a": "gp", "shared": "gp"}),
                json!({}),
            ),
            policy(
                "parent",
                Some("grandparent"),
                json!({"b": "p", "shared": "p"}),
                json!({}),
            ),
            policy(
                "child",
                Some("parent"),
                json!({"c": "c", "shared": "c"}),
                json!({}),
            ),
        ];
        let map = by_profile(&policies);
        let chain = resolve_chain(&map, "child").unwrap();
        let (enforced, _) = flatten(&chain);
        assert_eq!(enforced.get("a"), Some(&json!("gp")));
        assert_eq!(enforced.get("b"), Some(&json!("p")));
        assert_eq!(enforced.get("c"), Some(&json!("c")));
        assert_eq!(enforced.get("shared"), Some(&json!("c")));
    }
}
