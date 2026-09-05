// tests/unit/config_roaming_filter.rs
use cli_framework::config::managed::filter_user_scoped;
use cli_framework::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
use serde_json::{json, Map, Value};

fn field(key: &str, scope: Scope, local_only: bool, secret: bool) -> FieldManifest {
    FieldManifest {
        key: key.to_string(),
        kind: FieldKind::Str,
        default: None,
        label: None,
        description: None,
        group: None,
        scope,
        platforms: Vec::new(),
        secret,
        local_only,
        protected: false,
        manageable: true,
        enforceable: true,
        restart_required: false,
        constraints: None,
    }
}

fn doc(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn a_plain_user_scoped_field_still_roams() {
    let manifest = ConfigManifest::new("demo", vec![field("theme", Scope::User, false, false)]);
    let filtered = filter_user_scoped(&manifest, &doc(&[("theme", json!("dark"))]));
    assert_eq!(filtered.get("theme"), Some(&json!("dark")));
}

#[test]
fn a_local_only_field_never_roams_even_when_user_scoped() {
    let manifest = ConfigManifest::new("demo", vec![field("install_id", Scope::User, true, false)]);
    let filtered = filter_user_scoped(&manifest, &doc(&[("install_id", json!("abc"))]));
    assert!(
        filtered.is_empty(),
        "a local_only field is bootstrap state for one machine; roaming it to \
         every device makes one identifier out of many: {filtered:?}"
    );
}

#[test]
fn a_secret_field_never_roams_even_when_user_scoped() {
    let manifest = ConfigManifest::new("demo", vec![field("token", Scope::User, false, true)]);
    let filtered = filter_user_scoped(&manifest, &doc(&[("token", json!("s3cr3t"))]));
    assert!(filtered.is_empty(), "got {filtered:?}");
}

#[test]
fn machine_and_org_scoped_fields_are_still_excluded() {
    let manifest = ConfigManifest::new(
        "demo",
        vec![
            field("endpoint", Scope::Machine, false, false),
            field("tenant", Scope::Org, false, false),
        ],
    );
    let filtered = filter_user_scoped(
        &manifest,
        &doc(&[("endpoint", json!("http://x")), ("tenant", json!("acme"))]),
    );
    assert!(filtered.is_empty());
}

#[test]
fn a_key_the_manifest_does_not_declare_is_dropped() {
    let manifest = ConfigManifest::new("demo", vec![field("theme", Scope::User, false, false)]);
    let filtered = filter_user_scoped(&manifest, &doc(&[("mystery", json!(1))]));
    assert!(filtered.is_empty());
}
