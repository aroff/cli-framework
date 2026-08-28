//! [`flatten_to_paths`]: turn a nested JSON document into the flat
//! dotted-path keyspace [`crate::config::resolution::resolve`] operates on.
//! [`unflatten_from_paths`] is its counterpart, turning a resolved flat
//! document back into the nested shape `serde_json::from_value::<T>` needs.

use crate::config::manifest::{ConfigManifest, FieldKind, FieldManifest};
use serde_json::{Map, Value};

/// Flatten a JSON object into dot-joined leaf paths, matching
/// [`crate::config::manifest::ConfigManifest::iter_leaves`]'s path scheme.
///
/// A local config file, once loaded as a typed `T` and serialized back to
/// JSON (e.g. via `ConfigHandle::current_json`), is a fully nested document
/// mirroring the manifest's section structure — `{"network": {"proxy_url":
/// "..."}}`. The resolver needs the same document keyed as
/// `{"network.proxy_url": "..."}`. Arrays and non-object scalars are treated
/// as leaves and never descended into (a `list`-kind field's value is the
/// whole array, not one entry per index).
///
/// A non-object top-level `value` (which should not occur for a real config
/// document, since every [`super::super::VersionedConfig`] is a struct) flattens
/// to an empty map.
pub fn flatten_to_paths(value: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    if let Value::Object(map) = value {
        flatten_into(map, "", &mut out);
    }
    out
}

fn flatten_into(map: &Map<String, Value>, prefix: &str, out: &mut Map<String, Value>) {
    for (key, value) in map {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            Value::Object(nested) => flatten_into(nested, &path, out),
            other => {
                out.insert(path, other.clone());
            }
        }
    }
}

/// The inverse of [`flatten_to_paths`]: turn a flat dotted-path map (what
/// [`crate::config::resolution::resolve`]'s [`super::Resolved::entries`]
/// produces) back into the nested JSON shape `serde_json::from_value::<T>`
/// needs.
///
/// Reconstruction is guided by `manifest`'s own field structure — via
/// [`ConfigManifest::iter_leaves`]'s exact recursion, not a path-string-split
/// heuristic — because only the manifest actually knows which dotted
/// segments are section boundaries versus part of a leaf's own value (a
/// `list`-kind field's value is a whole JSON array under one flat key, never
/// one entry per dotted index) and which fields exist at all. A leaf present
/// in the manifest but absent from `flat` (nothing resolved for it — should
/// not occur for output produced by [`crate::config::resolution::resolve`],
/// which always resolves every leaf field) is simply omitted from the
/// rebuilt document rather than inserted as `null`, leaving `serde_json`'s
/// own default/missing-field handling for `T` to decide what happens next.
pub fn unflatten_from_paths(manifest: &ConfigManifest, flat: &Map<String, Value>) -> Value {
    Value::Object(unflatten_fields(&manifest.fields, "", flat))
}

fn unflatten_fields(
    fields: &[FieldManifest],
    prefix: &str,
    flat: &Map<String, Value>,
) -> Map<String, Value> {
    let mut out = Map::new();
    for field in fields {
        let path = if prefix.is_empty() {
            field.key.clone()
        } else {
            format!("{prefix}.{}", field.key)
        };
        match &field.kind {
            FieldKind::Section { fields: nested } => {
                out.insert(
                    field.key.clone(),
                    Value::Object(unflatten_fields(nested, &path, flat)),
                );
            }
            _ => {
                if let Some(value) = flat.get(&path) {
                    out.insert(field.key.clone(), value.clone());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattens_nested_objects_to_dotted_paths() {
        let value = json!({
            "top": true,
            "network": {
                "proxy_url": "http://proxy",
                "port": 8080,
            }
        });
        let flat = flatten_to_paths(&value);
        assert_eq!(flat.get("top"), Some(&json!(true)));
        assert_eq!(flat.get("network.proxy_url"), Some(&json!("http://proxy")));
        assert_eq!(flat.get("network.port"), Some(&json!(8080)));
        assert_eq!(flat.len(), 3);
    }

    #[test]
    fn arrays_are_leaves_not_descended_into() {
        let value = json!({ "tags": ["a", "b"] });
        let flat = flatten_to_paths(&value);
        assert_eq!(flat.get("tags"), Some(&json!(["a", "b"])));
    }

    #[test]
    fn non_object_top_level_flattens_to_empty() {
        assert!(flatten_to_paths(&json!("just a string")).is_empty());
        assert!(flatten_to_paths(&json!(42)).is_empty());
    }

    #[test]
    fn empty_object_flattens_to_empty_map() {
        assert!(flatten_to_paths(&json!({})).is_empty());
    }

    // ── unflatten_from_paths ─────────────────────────────────────────────────

    use crate::config::manifest::Scope;

    fn leaf(key: &str, kind: FieldKind) -> FieldManifest {
        FieldManifest {
            key: key.to_string(),
            kind,
            default: None,
            label: None,
            description: None,
            group: None,
            scope: Scope::Machine,
            platforms: vec![],
            secret: false,
            local_only: false,
            protected: false,
            manageable: true,
            enforceable: true,
            restart_required: false,
            constraints: None,
        }
    }

    fn network_manifest() -> ConfigManifest {
        ConfigManifest::new(
            "app",
            vec![
                leaf("top", FieldKind::Bool),
                FieldManifest {
                    kind: FieldKind::Section {
                        fields: vec![
                            leaf("proxy_url", FieldKind::Url),
                            leaf("port", FieldKind::Int),
                        ],
                    },
                    ..leaf("network", FieldKind::Bool)
                },
            ],
        )
    }

    #[test]
    fn unflatten_is_the_precise_inverse_of_flatten_for_a_nested_document() {
        let original = json!({
            "top": true,
            "network": {
                "proxy_url": "http://proxy",
                "port": 8080,
            }
        });

        let manifest = network_manifest();
        let flat = flatten_to_paths(&original);
        let rebuilt = unflatten_from_paths(&manifest, &flat);

        assert_eq!(rebuilt, original);
    }

    #[test]
    fn unflatten_of_an_empty_flat_map_omits_every_field() {
        let manifest = ConfigManifest::new("app", vec![leaf("greeting", FieldKind::Str)]);
        let rebuilt = unflatten_from_paths(&manifest, &Map::new());
        assert_eq!(rebuilt, json!({}));
    }

    #[test]
    fn unflatten_treats_a_list_leaf_value_as_a_single_whole_array_not_indices() {
        let manifest = ConfigManifest::new(
            "app",
            vec![leaf(
                "tags",
                FieldKind::List {
                    item: Box::new(FieldKind::Str),
                },
            )],
        );
        let mut flat = Map::new();
        flat.insert("tags".to_string(), json!(["a", "b"]));
        let rebuilt = unflatten_from_paths(&manifest, &flat);
        assert_eq!(rebuilt, json!({"tags": ["a", "b"]}));
    }

    #[test]
    fn unflatten_recovers_only_manifest_declared_fields_dropping_extras_in_the_flat_map() {
        // A flat map may carry keys the manifest doesn't declare (e.g. a
        // stray/old field) — unflatten walks the manifest's own field list,
        // so anything not declared there is simply never looked up.
        let manifest = ConfigManifest::new("app", vec![leaf("greeting", FieldKind::Str)]);
        let mut flat = Map::new();
        flat.insert("greeting".to_string(), json!("hi"));
        flat.insert("undeclared".to_string(), json!("should not appear"));
        let rebuilt = unflatten_from_paths(&manifest, &flat);
        assert_eq!(rebuilt, json!({"greeting": "hi"}));
    }
}
