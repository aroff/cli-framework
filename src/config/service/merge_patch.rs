//! [`merge_patch`]: RFC 7386 JSON Merge Patch, hand-rolled (spec 023 —
//! "Partial updates" — no `json-patch`/`merge-patch` crate exists anywhere
//! in this workspace, and the algorithm itself is short enough that adding
//! one would cost more than it saves).
//!
//! Applied per-tree by [`super::admin_router`]'s PATCH handler: the request
//! body addresses `enforced` and `recommended` as two independent merge-patch
//! fragments (so moving a field between the two trees is one request), plus
//! direct field sets for the scalar `parent_profile`/`max_cache_age_secs`/
//! `stale_action` keys — see that module's docs for the full body shape.

use serde_json::{Map, Value};

/// Apply RFC 7386 JSON Merge Patch: `patch` describes changes to make to
/// `target`, mutated in place.
///
/// - If `patch` is not a JSON object, it wholesale-replaces `target`.
/// - If `patch` is an object, `target` is coerced to an (empty, if it
///   wasn't already one) object, and each key in `patch` is applied in
///   turn: a `null` value removes that key from `target`; any other value
///   is merged recursively into whatever is currently at that key (or
///   inserted fresh, if `target` doesn't have that key yet).
pub fn merge_patch(target: &mut Value, patch: &Value) {
    if let Value::Object(patch_obj) = patch {
        if !target.is_object() {
            *target = Value::Object(Map::new());
        }
        let target_obj = target.as_object_mut().unwrap();
        for (key, patch_value) in patch_obj {
            if patch_value.is_null() {
                target_obj.remove(key);
            } else {
                let entry = target_obj.entry(key.clone()).or_insert(Value::Null);
                merge_patch(entry, patch_value);
            }
        }
    } else {
        *target = patch.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_in_the_patch_removes_the_key() {
        let mut target = json!({"a": 1, "b": 2});
        merge_patch(&mut target, &json!({"b": null}));
        assert_eq!(target, json!({"a": 1}));
    }

    #[test]
    fn a_new_scalar_key_is_inserted() {
        let mut target = json!({"a": 1});
        merge_patch(&mut target, &json!({"b": 2}));
        assert_eq!(target, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn an_existing_scalar_key_is_overwritten() {
        let mut target = json!({"a": 1});
        merge_patch(&mut target, &json!({"a": 2}));
        assert_eq!(target, json!({"a": 2}));
    }

    #[test]
    fn nested_objects_merge_recursively_leaving_untouched_siblings_alone() {
        let mut target = json!({"a": {"x": 1, "y": 2}, "b": 3});
        merge_patch(&mut target, &json!({"a": {"y": 20}}));
        assert_eq!(target, json!({"a": {"x": 1, "y": 20}, "b": 3}));
    }

    #[test]
    fn a_non_object_patch_wholesale_replaces_the_target_even_if_the_target_was_an_object() {
        let mut target = json!({"a": 1, "b": 2});
        merge_patch(&mut target, &json!("just a string now"));
        assert_eq!(target, json!("just a string now"));
    }

    #[test]
    fn a_non_object_patch_wholesale_replaces_a_scalar_target() {
        let mut target = json!(5);
        merge_patch(&mut target, &json!([1, 2, 3]));
        assert_eq!(target, json!([1, 2, 3]));
    }

    #[test]
    fn patching_an_absent_key_inserts_it_even_into_a_non_object_target() {
        let mut target = Value::Null;
        merge_patch(&mut target, &json!({"a": 1}));
        assert_eq!(target, json!({"a": 1}));
    }

    #[test]
    fn null_patch_value_for_a_key_that_never_existed_is_a_no_op() {
        let mut target = json!({"a": 1});
        merge_patch(&mut target, &json!({"ghost": null}));
        assert_eq!(target, json!({"a": 1}));
    }

    #[test]
    fn an_empty_object_patch_changes_nothing() {
        let mut target = json!({"a": 1, "b": {"c": 2}});
        merge_patch(&mut target, &json!({}));
        assert_eq!(target, json!({"a": 1, "b": {"c": 2}}));
    }

    #[test]
    fn nested_null_removes_a_key_deep_in_the_tree() {
        let mut target = json!({"a": {"x": 1, "y": 2}});
        merge_patch(&mut target, &json!({"a": {"x": null}}));
        assert_eq!(target, json!({"a": {"y": 2}}));
    }

    #[test]
    fn a_key_replacing_a_scalar_with_an_object_merges_into_a_fresh_object() {
        let mut target = json!({"a": 1});
        merge_patch(&mut target, &json!({"a": {"nested": true}}));
        assert_eq!(target, json!({"a": {"nested": true}}));
    }

    /// The RFC 7386 spec's own worked example, transcribed directly —
    /// pinning this implementation against the standard's canonical case,
    /// not just this crate's own hand-picked scenarios.
    #[test]
    fn rfc_7386_worked_example() {
        let mut target = json!({
            "a": "b",
            "c": {
                "d": "e",
                "f": "g"
            }
        });
        let patch = json!({
            "a": "z",
            "c": {
                "f": null
            }
        });
        merge_patch(&mut target, &patch);
        assert_eq!(
            target,
            json!({
                "a": "z",
                "c": {
                    "d": "e"
                }
            })
        );
    }
}
