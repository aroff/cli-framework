//! [`ConfigManifest`] data model — the wire/consumer-facing shape (spec 021, ADR 0073).
//!
//! Every type here is plain data (`Serialize`/`Deserialize`/`PartialEq`) on
//! purpose: a non-Rust application authors the identical JSON document by
//! hand, and every runtime consumer (the resolver, a provenance query, a
//! hypothetical non-Rust renderer) must be able to work from the document
//! alone. Nothing downstream may require the `#[derive(ConfigManifest)]`
//! macro's generated Rust type to function — see `crate::config::resolution`,
//! which resolves purely against `ConfigManifest` + `serde_json::Value`.

use serde::{Deserialize, Serialize};

/// The manifest document format version — independent of any application's
/// own [`super::super::VersionedConfig`] schema version. Bumped only when the
/// *shape of this document* changes (e.g. a new [`FieldKind`] variant), never
/// when an application adds or removes its own fields.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// An application's declared configuration surface (spec 021, "Config
/// manifest"): every field, its type, default, label/description/grouping,
/// and policy flags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigManifest {
    pub manifest_schema_version: u32,
    pub app: String,
    pub fields: Vec<FieldManifest>,
}

impl ConfigManifest {
    /// Construct a manifest stamped with the current
    /// [`MANIFEST_SCHEMA_VERSION`]. Mirrors the shape the derive macro emits,
    /// for hand-authored (non-Rust, or test) manifests.
    pub fn new(app: impl Into<String>, fields: Vec<FieldManifest>) -> Self {
        Self {
            manifest_schema_version: MANIFEST_SCHEMA_VERSION,
            app: app.into(),
            fields,
        }
    }

    /// Flatten every leaf (non-[`FieldKind::Section`]) field into its
    /// dot-joined path (e.g. a field `proxy_url` nested in a section
    /// `network` flattens to `"network.proxy_url"`).
    ///
    /// This dotted-path leaf keyspace is the coordinate system every other
    /// piece of spec 021 resolves against: a [`super::super::Policy`]'s
    /// `enforced`/`recommended` trees, the roaming user-config document, and
    /// the local config file (once flattened via
    /// [`crate::config::resolution::flatten_to_paths`]) are all flat JSON
    /// objects keyed this way. Sections are structural/grouping metadata
    /// only — they never carry a value of their own.
    pub fn iter_leaves(&self) -> Vec<LeafField<'_>> {
        let mut out = Vec::new();
        for field in &self.fields {
            collect_leaves(field, "", &mut out);
        }
        out
    }

    /// The [`FieldManifest`] whose flattened dotted path equals `path`, if any.
    pub fn leaf_by_path(&self, path: &str) -> Option<&FieldManifest> {
        self.iter_leaves()
            .into_iter()
            .find(|l| l.path == path)
            .map(|l| l.field)
    }
}

fn collect_leaves<'a>(field: &'a FieldManifest, prefix: &str, out: &mut Vec<LeafField<'a>>) {
    let full_path = if prefix.is_empty() {
        field.key.clone()
    } else {
        format!("{prefix}.{}", field.key)
    };
    if let FieldKind::Section { fields } = &field.kind {
        for child in fields {
            collect_leaves(child, &full_path, out);
        }
    } else {
        out.push(LeafField {
            path: full_path,
            field,
        });
    }
}

/// A leaf field paired with its fully dot-joined path, produced by
/// [`ConfigManifest::iter_leaves`].
#[derive(Debug, Clone, PartialEq)]
pub struct LeafField<'a> {
    pub path: String,
    pub field: &'a FieldManifest,
}

/// One field's declaration inside a [`ConfigManifest`].
///
/// `key` is the field's own segment name (not the full dotted path — see
/// [`ConfigManifest::iter_leaves`] for that). The eight policy flags
/// (`scope`, `platforms`, `secret`, `local_only`, `protected`, `manageable`,
/// `enforceable`, `restart_required`) are exactly the flag list in spec 021 /
/// ADR 0073.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldManifest {
    pub key: String,
    #[serde(flatten)]
    pub kind: FieldKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Whose value this is: `machine`, `user`, or `org`. Orthogonal to
    /// enforced/recommended for `machine`/`user` — `org` is the one
    /// exception (see [`Scope`] docs).
    #[serde(default)]
    pub scope: Scope,
    /// Which platforms this field applies to. Empty means "all platforms" —
    /// there is deliberately no closed enum here (spec 021 leaves the set of
    /// platform names to the application).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    /// Never written into a config document or a policy — structurally
    /// impossible to carry secrets over the managed-config channel.
    #[serde(default)]
    pub secret: bool,
    /// Bootstrap-only: never set remotely (e.g. the config service's own
    /// address).
    #[serde(default)]
    pub local_only: bool,
    /// Only the application's own privileged surface may change it; no
    /// automated caller (including an org policy) may.
    #[serde(default)]
    pub protected: bool,
    /// Whether an organisation may set this field at all (as `recommended`
    /// *or* `enforced`). Defaults to `true`.
    #[serde(default = "default_true")]
    pub manageable: bool,
    /// Whether an organisation may place this field in `enforced`. A field
    /// may still be `recommended` when `false` — see ADR 0073's "on state is
    /// itself an act of standing consent" rationale. Defaults to `true`.
    #[serde(default = "default_true")]
    pub enforceable: bool,
    #[serde(default)]
    pub restart_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<FieldConstraints>,
}

fn default_true() -> bool {
    true
}

/// Where a field's value belongs, per ADR 0073.
///
/// `machine` and `user` are genuinely orthogonal to enforced/recommended —
/// an organisation may recommend *or* enforce either. `org` is not a second
/// axis: an org-scoped field has no local existence to recommend a default
/// over, so it is always delivered `enforced`; a policy placing one in
/// `recommended` is invalid (rejected server-side, and dropped
/// defence-in-depth by the client — see `crate::config::resolution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Machine,
    User,
    Org,
}

/// Range and allowed-value constraints (spec 021 user story 6). Deliberately
/// small: just what both a renderer and a server need to reject bad values
/// before they reach application code.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldConstraints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<serde_json::Value>>,
}

/// The closed set of field kinds a manifest may declare (spec 021: "Field
/// kinds are deliberately few"). Internally tagged on `"kind"` (via
/// [`FieldManifest`]'s `#[serde(flatten)]`) so the wire shape for a boolean
/// field is `{"key": "...", "kind": "boolean", ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum FieldKind {
    #[serde(rename = "boolean")]
    Bool,
    #[serde(rename = "integer")]
    Int,
    #[serde(rename = "float")]
    Float,
    #[serde(rename = "string")]
    Str,
    #[serde(rename = "enumeration")]
    Enum { values: Vec<String> },
    /// Whole seconds. `std::time::Duration` has no built-in `Serialize`, so
    /// the wire (and Rust-field) representation for a duration is always a
    /// plain integer — see the derive macro's `#[manifest(kind = "duration")]`
    /// override.
    #[serde(rename = "duration")]
    Duration,
    #[serde(rename = "path")]
    Path,
    #[serde(rename = "url")]
    Url,
    #[serde(rename = "list")]
    List { item: Box<FieldKind> },
    /// A nested grouping of fields. Carries no value of its own — see
    /// [`ConfigManifest::iter_leaves`].
    #[serde(rename = "section")]
    Section { fields: Vec<FieldManifest> },
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn iter_leaves_flattens_nested_sections_with_dotted_paths() {
        let manifest = ConfigManifest::new(
            "myapp",
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
        );

        let leaves = manifest.iter_leaves();
        let paths: Vec<&str> = leaves.iter().map(|l| l.path.as_str()).collect();
        assert_eq!(paths, vec!["top", "network.proxy_url", "network.port"]);
    }

    #[test]
    fn leaf_by_path_finds_nested_field() {
        let manifest = ConfigManifest::new(
            "myapp",
            vec![FieldManifest {
                kind: FieldKind::Section {
                    fields: vec![leaf("proxy_url", FieldKind::Url)],
                },
                ..leaf("network", FieldKind::Bool)
            }],
        );
        assert!(manifest.leaf_by_path("network.proxy_url").is_some());
        assert!(manifest.leaf_by_path("network.missing").is_none());
        assert!(
            manifest.leaf_by_path("network").is_none(),
            "a section itself is not a leaf"
        );
    }

    #[test]
    fn scope_defaults_to_machine() {
        assert_eq!(Scope::default(), Scope::Machine);
    }

    #[test]
    fn manageable_and_enforceable_default_to_true_when_omitted_from_json() {
        // A hand-authored (non-Rust) manifest that omits these two flags
        // entirely must still get the safe default (`true`), matching what
        // the derive macro always stamps explicitly.
        let value = serde_json::json!({
            "key": "f",
            "kind": "boolean",
            "scope": "machine",
        });
        let field: FieldManifest = serde_json::from_value(value).unwrap();
        assert!(field.manageable);
        assert!(field.enforceable);
    }

    #[test]
    fn field_kind_json_roundtrip_for_every_kind() {
        let kinds = vec![
            FieldKind::Bool,
            FieldKind::Int,
            FieldKind::Float,
            FieldKind::Str,
            FieldKind::Enum {
                values: vec!["a".into(), "b".into()],
            },
            FieldKind::Duration,
            FieldKind::Path,
            FieldKind::Url,
            FieldKind::List {
                item: Box::new(FieldKind::Str),
            },
            FieldKind::Section {
                fields: vec![leaf("x", FieldKind::Bool)],
            },
        ];
        for kind in kinds {
            let field = leaf("f", kind.clone());
            let json = serde_json::to_value(&field).unwrap();
            let back: FieldManifest = serde_json::from_value(json).unwrap();
            assert_eq!(back.kind, kind);
        }
    }

    #[test]
    fn manifest_json_round_trips() {
        let manifest = ConfigManifest::new("myapp", vec![leaf("greeting", FieldKind::Str)]);
        let json = serde_json::to_value(&manifest).unwrap();
        assert_eq!(json["app"], "myapp");
        assert_eq!(json["manifest_schema_version"], MANIFEST_SCHEMA_VERSION);
        let back: ConfigManifest = serde_json::from_value(json).unwrap();
        assert_eq!(back, manifest);
    }
}
