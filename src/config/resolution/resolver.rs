//! [`resolve`]: fold a [`ConfigManifest`] and the six local/remote layers
//! into resolved values plus [`Provenance`] (spec 021, "Resolution order").
//!
//! ```text
//! defaults -> recommended -> config file -> environment -> flags -> builder overrides -> ENFORCED
//! ```
//!
//! `Enforced` is applied **last, as a veto pass** over whatever the first six
//! layers produced — not as a seventh layer in a simple stack. This is what
//! lets it beat environment variables and command-line flags even though
//! those are "more local" than a server-delivered policy.
//!
//! This module works entirely from [`ConfigManifest`] + `serde_json::Value` —
//! never from the Rust type a `#[derive(ConfigManifest)]` struct was applied
//! to. That is deliberate (spec 021, "Manifest schema is data, not Rust
//! types, at the consumption boundary"): a hand-authored manifest from a
//! non-Rust application resolves through the exact same code path.

use super::{Layer, Provenance};
use crate::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The six layers below the enforced veto pass, each a flat JSON object
/// keyed by the manifest's dotted leaf paths (see
/// [`ConfigManifest::iter_leaves`] and [`super::flatten_to_paths`]).
///
/// `recommended` and `enforced` are ordinarily a [`crate::config::Policy`]'s
/// two trees verbatim; the remaining four are local to this device/process.
#[derive(Debug, Clone, Default)]
pub struct ResolutionInput {
    pub recommended: Map<String, Value>,
    pub config_file: Map<String, Value>,
    pub environment: Map<String, Value>,
    pub flags: Map<String, Value>,
    pub builder_overrides: Map<String, Value>,
    pub enforced: Map<String, Value>,
}

/// Why a value present in a server tree (`recommended` or `enforced`) was
/// dropped instead of applied. Every drop is paired with exactly one of
/// these — "warned about, not applied" (spec 021) is made concrete here as a
/// structured reason a test can assert on, not a free-text log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningReason {
    /// The field is `local_only`; a server tree may never set it.
    LocalOnlyInServerTree,
    /// The field is not `manageable`; an organisation may not set it at all.
    NotManageableInServerTree,
    /// The field is `secret`; it can never appear in a policy document.
    SecretInServerTree,
    /// An `org`-scoped field appeared in `recommended`. `org` fields have no
    /// local existence to recommend a default over — they may only be
    /// `enforced`. The server is expected to reject this already (PRD 022);
    /// this is the client's defence-in-depth copy of that same rule.
    OrgScopeInRecommended,
    /// The field is `enforceable: false`; it may be `recommended` but never
    /// `enforced`.
    NotEnforceableInEnforced,
    /// The tree contains a key with no corresponding manifest field at all —
    /// tolerated for forward/backward version skew (spec 021 user story 38).
    UnknownKey,
    /// The tree contains a value whose JSON shape doesn't match the field's
    /// declared [`FieldKind`].
    TypeMismatch,
}

/// One dropped-and-warned-about entry, produced instead of applying a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionWarning {
    pub path: String,
    pub reason: WarningReason,
}

/// The output of [`resolve`]: a resolved value plus [`Provenance`] for every
/// leaf field in the manifest, and the warnings produced along the way.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    values: BTreeMap<String, Value>,
    provenance: BTreeMap<String, Provenance>,
    pub warnings: Vec<ResolutionWarning>,
}

impl Resolved {
    /// The resolved value for `path`, or `None` if `path` is not a leaf field
    /// in the manifest this was resolved against.
    pub fn value(&self, path: &str) -> Option<&Value> {
        self.values.get(path)
    }

    /// [`Provenance`] for `path`, or `None` if `path` is not a leaf field in
    /// the manifest this was resolved against.
    pub fn provenance(&self, path: &str) -> Option<Provenance> {
        self.provenance.get(path).copied()
    }

    /// Every resolved leaf path, in manifest order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    /// True if `resolve` recorded at least one warning for `path`.
    pub fn has_warning(&self, path: &str, reason: &WarningReason) -> bool {
        self.warnings
            .iter()
            .any(|w| w.path == path && &w.reason == reason)
    }

    /// Every resolved leaf as one `(path, value, provenance)` entry, in
    /// path order.
    ///
    /// A renderer (the built-in `config show` command, a settings UI) wants
    /// the whole resolved surface at once rather than combining
    /// [`Self::paths`] with a [`Self::value`]/[`Self::provenance`] lookup
    /// per path — this is that convenience, added for spec 021's "Command
    /// surface" (`config show`).
    pub fn entries(&self) -> Vec<ResolvedEntry> {
        self.values
            .iter()
            .map(|(path, value)| ResolvedEntry {
                path: path.clone(),
                value: value.clone(),
                provenance: self
                    .provenance
                    .get(path)
                    .copied()
                    .expect("resolve() always inserts a provenance entry alongside every value"),
            })
            .collect()
    }
}

/// One resolved leaf field's path, value, and [`Provenance`] together — see
/// [`Resolved::entries`].
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedEntry {
    pub path: String,
    pub value: Value,
    pub provenance: Provenance,
}

/// Resolve `manifest`'s leaf fields against `input`, applying the enforced
/// veto pass last.
///
/// Every leaf field always gets a value and a [`Provenance`] — there is no
/// "unresolved" state. A field absent from every layer resolves to its
/// manifest default (or a kind-appropriate zero value, for a hand-authored
/// manifest that omitted `default`) with [`Layer::Default`] provenance.
pub fn resolve(manifest: &ConfigManifest, input: &ResolutionInput) -> Resolved {
    let leaves = manifest.iter_leaves();
    let mut resolved = Resolved::default();

    for leaf in &leaves {
        let path = leaf.path.as_str();
        let field = leaf.field;

        let mut value = field
            .default
            .clone()
            .unwrap_or_else(|| default_for_kind(&field.kind));
        let mut layer = Layer::Default;

        // 2. recommended
        if let Some(raw) = input.recommended.get(path) {
            match server_tree_drop_reason_recommended(field, raw) {
                Some(reason) => resolved.warnings.push(ResolutionWarning {
                    path: path.to_string(),
                    reason,
                }),
                None => {
                    value = raw.clone();
                    layer = Layer::Recommended;
                }
            }
        }

        // 3. config file
        if let Some(raw) = input.config_file.get(path) {
            value = raw.clone();
            layer = Layer::ConfigFile;
        }

        // 4. environment
        if let Some(raw) = input.environment.get(path) {
            value = raw.clone();
            layer = Layer::Environment;
        }

        // 5. flags
        if let Some(raw) = input.flags.get(path) {
            value = raw.clone();
            layer = Layer::Flags;
        }

        // 6. builder overrides
        if let Some(raw) = input.builder_overrides.get(path) {
            value = raw.clone();
            layer = Layer::BuilderOverride;
        }

        // 7. ENFORCED — a veto pass over whatever the above produced, not a
        // "next layer": it is checked last unconditionally, so it beats
        // config file, environment, flags, *and* builder overrides alike.
        if let Some(raw) = input.enforced.get(path) {
            match server_tree_drop_reason_enforced(field, raw) {
                Some(reason) => resolved.warnings.push(ResolutionWarning {
                    path: path.to_string(),
                    reason,
                }),
                None => {
                    value = raw.clone();
                    layer = Layer::Enforced;
                }
            }
        }

        resolved.values.insert(path.to_string(), value);
        resolved
            .provenance
            .insert(path.to_string(), Provenance::new(layer));
    }

    push_unknown_key_warnings(&leaves, &input.recommended, &mut resolved.warnings);
    push_unknown_key_warnings(&leaves, &input.enforced, &mut resolved.warnings);

    resolved
}

/// Why (if any) a `recommended`-tree entry for `field` must be dropped
/// rather than applied. Order matters only for which single reason is
/// reported when several would apply; the field is dropped either way.
fn server_tree_drop_reason_recommended(
    field: &FieldManifest,
    raw: &Value,
) -> Option<WarningReason> {
    if field.local_only {
        return Some(WarningReason::LocalOnlyInServerTree);
    }
    if !field.manageable {
        return Some(WarningReason::NotManageableInServerTree);
    }
    if field.secret {
        return Some(WarningReason::SecretInServerTree);
    }
    if field.scope == Scope::Org {
        return Some(WarningReason::OrgScopeInRecommended);
    }
    if !value_matches_kind(raw, &field.kind) {
        return Some(WarningReason::TypeMismatch);
    }
    None
}

/// Why (if any) an `enforced`-tree entry for `field` must be dropped rather
/// than applied.
fn server_tree_drop_reason_enforced(field: &FieldManifest, raw: &Value) -> Option<WarningReason> {
    if field.local_only {
        return Some(WarningReason::LocalOnlyInServerTree);
    }
    if !field.manageable {
        return Some(WarningReason::NotManageableInServerTree);
    }
    if field.secret {
        return Some(WarningReason::SecretInServerTree);
    }
    if !field.enforceable {
        return Some(WarningReason::NotEnforceableInEnforced);
    }
    if !value_matches_kind(raw, &field.kind) {
        return Some(WarningReason::TypeMismatch);
    }
    None
}

fn push_unknown_key_warnings(
    leaves: &[crate::config::manifest::LeafField<'_>],
    tree: &Map<String, Value>,
    warnings: &mut Vec<ResolutionWarning>,
) {
    for key in tree.keys() {
        if !leaves.iter().any(|l| l.path == *key) {
            warnings.push(ResolutionWarning {
                path: key.clone(),
                reason: WarningReason::UnknownKey,
            });
        }
    }
}

/// Whether `value`'s JSON shape matches `kind`, used to skip
/// type-mismatched keys in a server tree (spec 021 user story 38) rather
/// than let a malformed policy corrupt resolution.
fn value_matches_kind(value: &Value, kind: &FieldKind) -> bool {
    match kind {
        FieldKind::Bool => value.is_boolean(),
        FieldKind::Int | FieldKind::Duration => value.is_i64() || value.is_u64(),
        FieldKind::Float => value.is_number(),
        FieldKind::Str | FieldKind::Path | FieldKind::Url => value.is_string(),
        FieldKind::Enum { values } => value
            .as_str()
            .is_some_and(|s| values.iter().any(|v| v == s)),
        FieldKind::List { item } => value
            .as_array()
            .is_some_and(|arr| arr.iter().all(|e| value_matches_kind(e, item))),
        FieldKind::Section { .. } => value.is_object(),
    }
}

/// A kind-appropriate zero value, used only when a leaf field's manifest
/// entry omits `default` entirely (the derive macro always supplies one from
/// `Default::default()`; a hand-authored manifest might not).
fn default_for_kind(kind: &FieldKind) -> Value {
    match kind {
        FieldKind::Bool => Value::Bool(false),
        FieldKind::Int | FieldKind::Duration => Value::from(0),
        FieldKind::Float => Value::from(0.0),
        FieldKind::Str | FieldKind::Path | FieldKind::Url => Value::String(String::new()),
        FieldKind::Enum { values } => values
            .first()
            .map(|v| Value::String(v.clone()))
            .unwrap_or_else(|| Value::String(String::new())),
        FieldKind::List { .. } => Value::Array(vec![]),
        FieldKind::Section { .. } => Value::Object(Map::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::FieldConstraints;
    use serde_json::json;

    fn field(key: &str, kind: FieldKind, default: Value) -> FieldManifest {
        FieldManifest {
            key: key.to_string(),
            kind,
            default: Some(default),
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

    /// Minimal sanity check that the pieces wire together — the exhaustive
    /// precedence matrix and failure-mode coverage lives in
    /// `tests/unit/config_resolution.rs`, matching the house convention
    /// established for `ConfigStore` (see `versioned.rs`).
    #[test]
    fn unset_field_resolves_to_its_default() {
        let manifest = ConfigManifest::new(
            "app",
            vec![field("greeting", FieldKind::Str, json!("hello"))],
        );
        let resolved = resolve(&manifest, &ResolutionInput::default());
        assert_eq!(resolved.value("greeting"), Some(&json!("hello")));
        assert_eq!(
            resolved.provenance("greeting").unwrap().layer,
            Layer::Default
        );
        assert!(!resolved.provenance("greeting").unwrap().locked);
    }

    #[test]
    fn enforced_wins_over_every_local_layer_at_once() {
        let manifest = ConfigManifest::new(
            "app",
            vec![field("greeting", FieldKind::Str, json!("default"))],
        );
        let mut input = ResolutionInput::default();
        input
            .config_file
            .insert("greeting".to_string(), json!("from-file"));
        input
            .environment
            .insert("greeting".to_string(), json!("from-env"));
        input
            .flags
            .insert("greeting".to_string(), json!("from-flag"));
        input
            .builder_overrides
            .insert("greeting".to_string(), json!("from-builder"));
        input
            .enforced
            .insert("greeting".to_string(), json!("from-enforced"));

        let resolved = resolve(&manifest, &input);
        assert_eq!(resolved.value("greeting"), Some(&json!("from-enforced")));
        assert!(resolved.provenance("greeting").unwrap().locked);
    }

    #[test]
    fn unknown_kind_type_mismatch_is_dropped_with_warning() {
        let manifest = ConfigManifest::new("app", vec![field("count", FieldKind::Int, json!(1))]);
        let mut input = ResolutionInput::default();
        input
            .recommended
            .insert("count".to_string(), json!("not-a-number"));
        let resolved = resolve(&manifest, &input);
        assert_eq!(resolved.value("count"), Some(&json!(1)));
        assert!(resolved.has_warning("count", &WarningReason::TypeMismatch));
    }

    // `value_matches_kind` / `default_for_kind` are private pure-function
    // helpers exercised only for `Str`/`Int` through `resolve()` in the tests
    // above (and in `tests/unit/config_resolution.rs`) — every leaf field
    // used there happens to be one of those two kinds. In particular
    // `FieldKind::Section` can never reach either function *through*
    // `resolve()` at all: `ConfigManifest::iter_leaves` filters section
    // fields out before the resolver ever sees them (a section carries no
    // value of its own). Tested directly here instead, matching the house
    // convention for small pure-function helpers (see `format.rs`'s inline
    // tests for `bytes_to_value`/`value_to_bytes`).
    #[test]
    fn value_matches_kind_covers_every_field_kind() {
        assert!(value_matches_kind(&json!(true), &FieldKind::Bool));
        assert!(!value_matches_kind(&json!("x"), &FieldKind::Bool));
        assert!(value_matches_kind(&json!(1), &FieldKind::Int));
        assert!(value_matches_kind(&json!(1), &FieldKind::Duration));
        assert!(value_matches_kind(&json!(1.5), &FieldKind::Float));
        assert!(value_matches_kind(&json!("s"), &FieldKind::Str));
        assert!(value_matches_kind(&json!("/p"), &FieldKind::Path));
        assert!(value_matches_kind(&json!("http://x"), &FieldKind::Url));

        let enum_kind = FieldKind::Enum {
            values: vec!["a".to_string(), "b".to_string()],
        };
        assert!(value_matches_kind(&json!("a"), &enum_kind));
        assert!(!value_matches_kind(&json!("z"), &enum_kind));

        let list_kind = FieldKind::List {
            item: Box::new(FieldKind::Int),
        };
        assert!(value_matches_kind(&json!([1, 2]), &list_kind));
        assert!(!value_matches_kind(&json!([1, "x"]), &list_kind));
        assert!(!value_matches_kind(&json!("not-a-list"), &list_kind));

        let section_kind = FieldKind::Section { fields: vec![] };
        assert!(value_matches_kind(&json!({}), &section_kind));
        assert!(!value_matches_kind(&json!([]), &section_kind));
    }

    #[test]
    fn default_for_kind_covers_every_field_kind() {
        assert_eq!(default_for_kind(&FieldKind::Bool), json!(false));
        assert_eq!(default_for_kind(&FieldKind::Int), json!(0));
        assert_eq!(default_for_kind(&FieldKind::Duration), json!(0));
        assert_eq!(default_for_kind(&FieldKind::Float), json!(0.0));
        assert_eq!(default_for_kind(&FieldKind::Str), json!(""));
        assert_eq!(default_for_kind(&FieldKind::Path), json!(""));
        assert_eq!(default_for_kind(&FieldKind::Url), json!(""));
        assert_eq!(
            default_for_kind(&FieldKind::Enum {
                values: vec!["first".to_string(), "second".to_string()]
            }),
            json!("first")
        );
        assert_eq!(
            default_for_kind(&FieldKind::Enum { values: vec![] }),
            json!("")
        );
        assert_eq!(
            default_for_kind(&FieldKind::List {
                item: Box::new(FieldKind::Str)
            }),
            json!([])
        );
        assert_eq!(
            default_for_kind(&FieldKind::Section { fields: vec![] }),
            json!({})
        );
    }

    /// A leaf field with no `default` at all (as a hand-authored, non-Rust
    /// manifest might omit) falls back to `default_for_kind` through the
    /// real `resolve()` entry point, not just the direct unit test above.
    #[test]
    fn field_with_no_manifest_default_falls_back_through_resolve() {
        let mut f = field("count", FieldKind::Int, json!(0));
        f.default = None;
        let manifest = ConfigManifest::new("app", vec![f]);
        let resolved = resolve(&manifest, &ResolutionInput::default());
        assert_eq!(resolved.value("count"), Some(&json!(0)));
    }

    #[test]
    fn entries_pairs_every_path_with_its_value_and_provenance() {
        let manifest = ConfigManifest::new(
            "app",
            vec![
                field("a", FieldKind::Str, json!("a-default")),
                field("b", FieldKind::Str, json!("b-default")),
            ],
        );
        let mut input = ResolutionInput::default();
        input.enforced.insert("b".to_string(), json!("b-enforced"));

        let resolved = resolve(&manifest, &input);
        let mut entries = resolved.entries();
        entries.sort_by(|x, y| x.path.cmp(&y.path));

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "a");
        assert_eq!(entries[0].value, json!("a-default"));
        assert_eq!(entries[0].provenance.layer, Layer::Default);
        assert!(!entries[0].provenance.locked);

        assert_eq!(entries[1].path, "b");
        assert_eq!(entries[1].value, json!("b-enforced"));
        assert_eq!(entries[1].provenance.layer, Layer::Enforced);
        assert!(entries[1].provenance.locked);
    }

    #[test]
    fn resolved_paths_lists_every_leaf() {
        let manifest = manifest_with_two_fields();
        let resolved = resolve(&manifest, &ResolutionInput::default());
        let mut paths: Vec<&str> = resolved.paths().collect();
        paths.sort_unstable();
        assert_eq!(paths, vec!["a", "b"]);
    }

    fn manifest_with_two_fields() -> ConfigManifest {
        ConfigManifest::new(
            "app",
            vec![
                field("a", FieldKind::Str, json!("")),
                field("b", FieldKind::Str, json!("")),
            ],
        )
    }

    #[test]
    fn constraints_are_carried_but_not_enforced_by_the_resolver() {
        // Constraints are advisory metadata for renderers/servers (spec 021
        // user story 6); the resolver itself does not reject an
        // out-of-range value — that is the server/renderer's job. Documented
        // here so the boundary is explicit and tested, not merely assumed.
        let mut f = field("count", FieldKind::Int, json!(1));
        f.constraints = Some(FieldConstraints {
            min: Some(0.0),
            max: Some(10.0),
            allowed_values: None,
        });
        let manifest = ConfigManifest::new("app", vec![f]);
        let mut input = ResolutionInput::default();
        input.flags.insert("count".to_string(), json!(9999));
        let resolved = resolve(&manifest, &input);
        assert_eq!(resolved.value("count"), Some(&json!(9999)));
    }
}
