//! One-way export of a [`super::ConfigManifest`] to JSON Schema.
//!
//! Spec 021: "A one-way export to JSON Schema is provided for external
//! validators and is consumed by nothing here." This module exists purely so
//! an external tool (an editor, a settings UI, a CI validator in another
//! language) can validate a candidate document shape — nothing in this crate
//! reads its own output back.

use super::{FieldConstraints, FieldKind, FieldManifest};
use serde_json::{json, Map, Value};

/// Render `manifest` as a JSON Schema (draft-07) document describing the
/// flattened field surface. Sections become nested `object` schemas; every
/// other [`FieldKind`] maps to the obvious JSON Schema primitive.
pub fn to_json_schema(manifest: &super::ConfigManifest) -> Value {
    let properties: Map<String, Value> = manifest
        .fields
        .iter()
        .map(|f| (f.key.clone(), field_schema(f)))
        .collect();

    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": manifest.app,
        "type": "object",
        "properties": properties,
    })
}

fn field_schema(field: &FieldManifest) -> Value {
    let mut schema = kind_schema(&field.kind);
    if let Some(obj) = schema.as_object_mut() {
        if let Some(label) = &field.label {
            obj.insert("title".to_string(), json!(label));
        }
        if let Some(description) = &field.description {
            obj.insert("description".to_string(), json!(description));
        }
        apply_constraints(obj, field.constraints.as_ref());
    }
    schema
}

fn apply_constraints(obj: &mut Map<String, Value>, constraints: Option<&FieldConstraints>) {
    let Some(c) = constraints else { return };
    if let Some(min) = c.min {
        obj.insert("minimum".to_string(), json!(min));
    }
    if let Some(max) = c.max {
        obj.insert("maximum".to_string(), json!(max));
    }
    if let Some(allowed) = &c.allowed_values {
        obj.insert("enum".to_string(), json!(allowed));
    }
}

/// Schema for a bare [`FieldKind`], used both for top-level fields and for
/// list item kinds (which have no [`FieldManifest`] context of their own).
fn kind_schema(kind: &FieldKind) -> Value {
    match kind {
        FieldKind::Bool => json!({"type": "boolean"}),
        FieldKind::Int | FieldKind::Duration => json!({"type": "integer"}),
        FieldKind::Float => json!({"type": "number"}),
        FieldKind::Str | FieldKind::Path | FieldKind::Url => json!({"type": "string"}),
        FieldKind::Enum { values } => json!({"type": "string", "enum": values}),
        FieldKind::List { item } => json!({"type": "array", "items": kind_schema(item)}),
        FieldKind::Section { fields } => {
            let properties: Map<String, Value> = fields
                .iter()
                .map(|f| (f.key.clone(), field_schema(f)))
                .collect();
            json!({"type": "object", "properties": properties})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{ConfigManifest, Scope};

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

    #[test]
    fn maps_every_primitive_kind_to_expected_json_schema_type() {
        let cases = vec![
            (FieldKind::Bool, "boolean"),
            (FieldKind::Int, "integer"),
            (FieldKind::Duration, "integer"),
            (FieldKind::Float, "number"),
            (FieldKind::Str, "string"),
            (FieldKind::Path, "string"),
            (FieldKind::Url, "string"),
        ];
        for (kind, expected) in cases {
            let manifest = ConfigManifest::new("app", vec![leaf("f", kind)]);
            let schema = to_json_schema(&manifest);
            assert_eq!(schema["properties"]["f"]["type"], expected);
        }
    }

    #[test]
    fn enum_kind_carries_allowed_values() {
        let manifest = ConfigManifest::new(
            "app",
            vec![leaf(
                "f",
                FieldKind::Enum {
                    values: vec!["a".into(), "b".into()],
                },
            )],
        );
        let schema = to_json_schema(&manifest);
        assert_eq!(schema["properties"]["f"]["type"], "string");
        assert_eq!(schema["properties"]["f"]["enum"], json!(["a", "b"]));
    }

    #[test]
    fn list_kind_nests_item_schema() {
        let manifest = ConfigManifest::new(
            "app",
            vec![leaf(
                "f",
                FieldKind::List {
                    item: Box::new(FieldKind::Int),
                },
            )],
        );
        let schema = to_json_schema(&manifest);
        assert_eq!(schema["properties"]["f"]["type"], "array");
        assert_eq!(schema["properties"]["f"]["items"]["type"], "integer");
    }

    #[test]
    fn section_kind_nests_object_properties() {
        let manifest = ConfigManifest::new(
            "app",
            vec![FieldManifest {
                kind: FieldKind::Section {
                    fields: vec![leaf("proxy_url", FieldKind::Url)],
                },
                ..leaf("network", FieldKind::Bool)
            }],
        );
        let schema = to_json_schema(&manifest);
        assert_eq!(schema["properties"]["network"]["type"], "object");
        assert_eq!(
            schema["properties"]["network"]["properties"]["proxy_url"]["type"],
            "string"
        );
    }

    #[test]
    fn label_description_and_constraints_are_rendered() {
        let mut field = leaf("f", FieldKind::Int);
        field.label = Some("Friendly Label".to_string());
        field.description = Some("A description".to_string());
        field.constraints = Some(FieldConstraints {
            min: Some(1.0),
            max: Some(10.0),
            allowed_values: None,
        });
        let manifest = ConfigManifest::new("app", vec![field]);
        let schema = to_json_schema(&manifest);
        let f = &schema["properties"]["f"];
        assert_eq!(f["title"], "Friendly Label");
        assert_eq!(f["description"], "A description");
        assert_eq!(f["minimum"], 1.0);
        assert_eq!(f["maximum"], 10.0);
    }

    #[test]
    fn allowed_values_constraint_renders_as_json_schema_enum() {
        let mut field = leaf("f", FieldKind::Int);
        field.constraints = Some(FieldConstraints {
            min: None,
            max: None,
            allowed_values: Some(vec![json!(1), json!(2), json!(3)]),
        });
        let manifest = ConfigManifest::new("app", vec![field]);
        let schema = to_json_schema(&manifest);
        assert_eq!(schema["properties"]["f"]["enum"], json!([1, 2, 3]));
    }

    #[test]
    fn top_level_schema_has_app_title_and_object_type() {
        let manifest = ConfigManifest::new("myapp", vec![leaf("f", FieldKind::Bool)]);
        let schema = to_json_schema(&manifest);
        assert_eq!(schema["title"], "myapp");
        assert_eq!(schema["type"], "object");
        assert!(schema["$schema"]
            .as_str()
            .unwrap()
            .contains("json-schema.org"));
    }
}
