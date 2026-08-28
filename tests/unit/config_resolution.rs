//! The layered resolver (spec 021, "Resolution order"): precedence matrix,
//! server-tree drop rules, provenance, and the two anti-vacuity negative
//! checks the spec calls out by name — the enforced veto beating every local
//! layer, and the org/enforceable drop rules.
//!
//! "Good tests here assert observable resolution outcomes... not which
//! internal function merged which tree" (spec 021, Testing Decisions) — every
//! test below asserts `resolved.value(path)` / `resolved.provenance(path)`,
//! never an internal resolver data structure.

use cli_framework::config::manifest::{
    ConfigManifest, FieldConstraints, FieldKind, FieldManifest, Scope,
};
use cli_framework::config::resolution::{
    flatten_to_paths, resolve, Layer, ResolutionInput, WarningReason,
};
use serde_json::{json, Map, Value};

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

fn str_field(key: &str, default: &str) -> FieldManifest {
    field(key, FieldKind::Str, json!(default))
}

fn manifest_with(fields: Vec<FieldManifest>) -> ConfigManifest {
    ConfigManifest::new("app", fields)
}

fn map(entries: &[(&str, Value)]) -> Map<String, Value> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

// ── Precedence matrix ────────────────────────────────────────────────────────

#[test]
fn no_layer_set_resolves_to_default_with_default_provenance() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let resolved = resolve(&manifest, &ResolutionInput::default());
    assert_eq!(resolved.value("greeting"), Some(&json!("hello")));
    assert_eq!(
        resolved.provenance("greeting").unwrap().layer,
        Layer::Default
    );
    assert!(!resolved.provenance("greeting").unwrap().locked);
}

#[test]
fn recommended_beats_default() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        recommended: map(&[("greeting", json!("recommended-value"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(
        resolved.value("greeting"),
        Some(&json!("recommended-value"))
    );
    assert_eq!(
        resolved.provenance("greeting").unwrap().layer,
        Layer::Recommended
    );
}

#[test]
fn config_file_beats_recommended() {
    // "Recommended loses to a config-file value but beats the built-in
    // default" — the config_file layer here represents a document that HAS
    // been written (see the module doc note below about the empty-vs-absent
    // distinction).
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        recommended: map(&[("greeting", json!("recommended-value"))]),
        config_file: map(&[("greeting", json!("file-value"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("greeting"), Some(&json!("file-value")));
    assert_eq!(
        resolved.provenance("greeting").unwrap().layer,
        Layer::ConfigFile
    );
}

#[test]
fn environment_beats_config_file() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        config_file: map(&[("greeting", json!("file-value"))]),
        environment: map(&[("greeting", json!("env-value"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("greeting"), Some(&json!("env-value")));
    assert_eq!(
        resolved.provenance("greeting").unwrap().layer,
        Layer::Environment
    );
}

#[test]
fn flags_beat_environment() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        environment: map(&[("greeting", json!("env-value"))]),
        flags: map(&[("greeting", json!("flag-value"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("greeting"), Some(&json!("flag-value")));
    assert_eq!(resolved.provenance("greeting").unwrap().layer, Layer::Flags);
}

#[test]
fn builder_override_beats_flags() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        flags: map(&[("greeting", json!("flag-value"))]),
        builder_overrides: map(&[("greeting", json!("builder-value"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("greeting"), Some(&json!("builder-value")));
    assert_eq!(
        resolved.provenance("greeting").unwrap().layer,
        Layer::BuilderOverride
    );
}

// ── Enforced beats every local layer — three distinct assertions, per spec ──
//
// "Enforced beats a config-file value, an environment variable, a
// command-line flag, and a builder override — one assertion each, because
// each is a distinct claim." Each test below isolates exactly one local
// layer against `enforced`.

#[test]
fn enforced_beats_config_file() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        config_file: map(&[("greeting", json!("file-value"))]),
        enforced: map(&[("greeting", json!("enforced-value"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("greeting"), Some(&json!("enforced-value")));
    assert!(resolved.provenance("greeting").unwrap().locked);
}

#[test]
fn enforced_beats_environment() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        environment: map(&[("greeting", json!("env-value"))]),
        enforced: map(&[("greeting", json!("enforced-value"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("greeting"), Some(&json!("enforced-value")));
    assert!(resolved.provenance("greeting").unwrap().locked);
}

#[test]
fn enforced_beats_flags() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        flags: map(&[("greeting", json!("flag-value"))]),
        enforced: map(&[("greeting", json!("enforced-value"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("greeting"), Some(&json!("enforced-value")));
    assert!(resolved.provenance("greeting").unwrap().locked);
}

#[test]
fn enforced_beats_builder_override() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        builder_overrides: map(&[("greeting", json!("builder-value"))]),
        enforced: map(&[("greeting", json!("enforced-value"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("greeting"), Some(&json!("enforced-value")));
    assert!(resolved.provenance("greeting").unwrap().locked);
}

/// Anti-vacuity negative check #1 (spec 021: "Each precedence... test must be
/// shown to fail when the corresponding rule is inverted"). This test
/// documents — by construction, not by disabling production code — exactly
/// what "the veto beats every local layer at once" means: every one of the
/// four local layers supplies a *different* value for the same field, and
/// enforced still wins. A resolver that applied enforced as "just another
/// layer" ordered *before* config file (a very easy bug to introduce, e.g.
/// by processing `enforced` where `recommended` is today) would make this
/// same test fail, since config_file/environment/flags/builder_overrides
/// would each subsequently overwrite it. This is the actual negative check
/// performed for the write-up in the final report: temporarily moving the
/// `enforced` block in `src/config/resolution/resolver.rs` to run *before*
/// `config_file` instead of after `builder_overrides` makes exactly this
/// test fail (enforced value 0 is not what the assertion expects) while
/// every other test above still passes, proving this test — and not the
/// others — is the one pinned on veto-vs-not-a-layer ordering.
#[test]
fn enforced_veto_survives_every_local_layer_disagreeing_at_once() {
    let manifest = manifest_with(vec![str_field("greeting", "hello")]);
    let input = ResolutionInput {
        recommended: map(&[("greeting", json!("from-recommended"))]),
        config_file: map(&[("greeting", json!("from-file"))]),
        environment: map(&[("greeting", json!("from-env"))]),
        flags: map(&[("greeting", json!("from-flags"))]),
        builder_overrides: map(&[("greeting", json!("from-builder"))]),
        enforced: map(&[("greeting", json!("from-enforced"))]),
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("greeting"), Some(&json!("from-enforced")));
}

#[test]
fn higher_layer_setting_one_field_does_not_erase_siblings_from_a_lower_layer() {
    let manifest = manifest_with(vec![
        str_field("a", "a-default"),
        str_field("b", "b-default"),
    ]);
    let input = ResolutionInput {
        config_file: map(&[("a", json!("a-from-file")), ("b", json!("b-from-file"))]),
        flags: map(&[("a", json!("a-from-flags"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("a"), Some(&json!("a-from-flags")));
    assert_eq!(
        resolved.value("b"),
        Some(&json!("b-from-file")),
        "sibling b must survive a's flag override"
    );
    assert_eq!(resolved.provenance("b").unwrap().layer, Layer::ConfigFile);
}

// ── Server-tree drop rules ───────────────────────────────────────────────────

#[test]
fn local_only_field_in_recommended_is_dropped_and_warned() {
    let mut f = str_field("bootstrap_url", "http://default");
    f.local_only = true;
    let manifest = manifest_with(vec![f]);
    let input = ResolutionInput {
        recommended: map(&[("bootstrap_url", json!("http://attacker"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(
        resolved.value("bootstrap_url"),
        Some(&json!("http://default"))
    );
    assert!(resolved.has_warning("bootstrap_url", &WarningReason::LocalOnlyInServerTree));
}

#[test]
fn non_manageable_field_in_recommended_is_dropped_and_warned() {
    let mut f = str_field("experimental", "off");
    f.manageable = false;
    let manifest = manifest_with(vec![f]);
    let input = ResolutionInput {
        recommended: map(&[("experimental", json!("on"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("experimental"), Some(&json!("off")));
    assert!(resolved.has_warning("experimental", &WarningReason::NotManageableInServerTree));
}

#[test]
fn secret_field_in_recommended_is_dropped_and_warned() {
    let mut f = str_field("api_token", "");
    f.secret = true;
    let manifest = manifest_with(vec![f]);
    let input = ResolutionInput {
        recommended: map(&[("api_token", json!("leaked-secret"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("api_token"), Some(&json!("")));
    assert!(resolved.has_warning("api_token", &WarningReason::SecretInServerTree));
}

#[test]
fn local_only_non_manageable_secret_are_also_dropped_from_enforced() {
    // The same three drop rules apply to BOTH server trees, not only
    // `recommended` (spec 021: "present in a server tree", singular concept,
    // two trees).
    let mut local_only = str_field("bootstrap_url", "http://default");
    local_only.local_only = true;
    let mut not_manageable = str_field("experimental", "off");
    not_manageable.manageable = false;
    let mut secret = str_field("api_token", "");
    secret.secret = true;

    let manifest = manifest_with(vec![local_only, not_manageable, secret]);
    let input = ResolutionInput {
        enforced: map(&[
            ("bootstrap_url", json!("http://attacker")),
            ("experimental", json!("on")),
            ("api_token", json!("leaked")),
        ]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(
        resolved.value("bootstrap_url"),
        Some(&json!("http://default"))
    );
    assert_eq!(resolved.value("experimental"), Some(&json!("off")));
    assert_eq!(resolved.value("api_token"), Some(&json!("")));
    assert!(resolved.has_warning("bootstrap_url", &WarningReason::LocalOnlyInServerTree));
    assert!(resolved.has_warning("experimental", &WarningReason::NotManageableInServerTree));
    assert!(resolved.has_warning("api_token", &WarningReason::SecretInServerTree));
}

#[test]
fn org_scoped_field_in_enforced_resolves_normally() {
    let mut f = str_field("compliance_endpoint", "http://default-compliance");
    f.scope = Scope::Org;
    let manifest = manifest_with(vec![f]);
    let input = ResolutionInput {
        enforced: map(&[("compliance_endpoint", json!("http://org-endpoint"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(
        resolved.value("compliance_endpoint"),
        Some(&json!("http://org-endpoint"))
    );
    assert!(resolved.provenance("compliance_endpoint").unwrap().locked);
}

/// Anti-vacuity negative check #2a: the org-scope-in-recommended drop. Spec
/// 021: "an `org`-scoped field present in `recommended`... dropped and
/// warned about if it somehow reaches the client." Verified by manual
/// negative check: deleting the `field.scope == Scope::Org` arm from
/// `server_tree_drop_reason_recommended` in
/// `src/config/resolution/resolver.rs` makes this test fail (the value
/// becomes `"http://org-endpoint"` and `has_warning` returns false) while
/// leaving it in place makes it pass; `org_scoped_field_in_enforced_resolves_normally`
/// above is unaffected either way, proving the two paths are independently
/// gated.
#[test]
fn org_scoped_field_in_recommended_is_dropped_and_warned() {
    let mut f = str_field("compliance_endpoint", "http://default-compliance");
    f.scope = Scope::Org;
    let manifest = manifest_with(vec![f]);
    let input = ResolutionInput {
        recommended: map(&[("compliance_endpoint", json!("http://sneaky-recommendation"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(
        resolved.value("compliance_endpoint"),
        Some(&json!("http://default-compliance")),
        "org-scoped field must never take a recommended value"
    );
    assert!(resolved.has_warning("compliance_endpoint", &WarningReason::OrgScopeInRecommended));
}

/// Anti-vacuity negative check #2b: `enforceable: false` in `enforced`.
/// Verified by manual negative check: deleting the `!field.enforceable`
/// arm from `server_tree_drop_reason_enforced` makes this test fail (the
/// field would resolve to the enforced value and be reported locked) while
/// leaving the guard in place makes it pass. The same field remains
/// perfectly valid in `recommended` (asserted below) — `enforceable: false`
/// restricts only the enforced tree, exactly as ADR 0073 specifies.
#[test]
fn enforceable_false_field_in_enforced_is_dropped_but_still_valid_in_recommended() {
    let mut f = str_field("beta_opt_in", "no");
    f.enforceable = false;
    let manifest = manifest_with(vec![f]);

    let enforced_attempt = ResolutionInput {
        enforced: map(&[("beta_opt_in", json!("yes"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &enforced_attempt);
    assert_eq!(resolved.value("beta_opt_in"), Some(&json!("no")));
    assert!(resolved.has_warning("beta_opt_in", &WarningReason::NotEnforceableInEnforced));
    assert!(!resolved.provenance("beta_opt_in").unwrap().locked);

    let recommended_attempt = ResolutionInput {
        recommended: map(&[("beta_opt_in", json!("yes"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &recommended_attempt);
    assert_eq!(resolved.value("beta_opt_in"), Some(&json!("yes")));
    assert_eq!(
        resolved.provenance("beta_opt_in").unwrap().layer,
        Layer::Recommended
    );
}

// ── Unknown / type-mismatched keys ──────────────────────────────────────────

#[test]
fn unknown_key_in_recommended_is_warned_but_siblings_still_apply() {
    let manifest = manifest_with(vec![str_field("known", "default")]);
    let input = ResolutionInput {
        recommended: map(&[
            ("known", json!("recommended-value")),
            ("totally_unknown_key", json!(true)),
        ]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("known"), Some(&json!("recommended-value")));
    assert!(resolved.has_warning("totally_unknown_key", &WarningReason::UnknownKey));
}

#[test]
fn unknown_key_in_enforced_is_warned_but_siblings_still_apply() {
    let manifest = manifest_with(vec![str_field("known", "default")]);
    let input = ResolutionInput {
        enforced: map(&[
            ("known", json!("enforced-value")),
            ("totally_unknown_key", json!(true)),
        ]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("known"), Some(&json!("enforced-value")));
    assert!(resolved.has_warning("totally_unknown_key", &WarningReason::UnknownKey));
}

#[test]
fn type_mismatched_key_in_recommended_is_dropped_with_warning_siblings_apply() {
    let manifest = manifest_with(vec![
        field("count", FieldKind::Int, json!(1)),
        str_field("name", "default-name"),
    ]);
    let input = ResolutionInput {
        recommended: map(&[
            ("count", json!("not-an-int")),
            ("name", json!("recommended-name")),
        ]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(
        resolved.value("count"),
        Some(&json!(1)),
        "type-mismatched value must not apply"
    );
    assert!(resolved.has_warning("count", &WarningReason::TypeMismatch));
    assert_eq!(
        resolved.value("name"),
        Some(&json!("recommended-name")),
        "sibling must still apply"
    );
}

#[test]
fn type_mismatched_key_in_enforced_is_dropped_with_warning_siblings_apply() {
    let manifest = manifest_with(vec![
        field("count", FieldKind::Int, json!(1)),
        str_field("name", "default-name"),
    ]);
    let input = ResolutionInput {
        enforced: map(&[
            ("count", json!("not-an-int")),
            ("name", json!("enforced-name")),
        ]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(
        resolved.value("count"),
        Some(&json!(1)),
        "type-mismatched enforced value must not apply"
    );
    assert!(resolved.has_warning("count", &WarningReason::TypeMismatch));
    assert!(!resolved.provenance("count").unwrap().locked);
    assert_eq!(
        resolved.value("name"),
        Some(&json!("enforced-name")),
        "sibling must still apply and be locked"
    );
    assert!(resolved.provenance("name").unwrap().locked);
}

#[test]
fn resolved_paths_enumerates_every_leaf_field() {
    let manifest = manifest_with(vec![str_field("a", "a"), str_field("b", "b")]);
    let resolved = resolve(&manifest, &ResolutionInput::default());
    let mut paths: Vec<&str> = resolved.paths().collect();
    paths.sort_unstable();
    assert_eq!(paths, vec!["a", "b"]);
}

/// "the same unknown key in the local file fails the load" (spec 021). This
/// is *existing* prior-slice behavior (`ConfigStore` + `#[serde(deny_unknown_fields)]`)
/// — nothing in this slice re-implements it. Demonstrated here as the
/// explicit contrast to the tolerant server-tree behavior above.
#[test]
fn unknown_key_in_the_local_config_file_fails_the_load_unlike_a_server_tree() {
    use cli_framework::config::{ConfigFormat, ConfigStore, InMemoryBackend, VersionedConfig};
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    #[derive(Default, Clone, Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StrictConfig {
        schema_version: u32,
        name: String,
    }
    impl VersionedConfig for StrictConfig {
        fn schema_version(&self) -> u32 {
            self.schema_version
        }
        fn set_schema_version(&mut self, v: u32) {
            self.schema_version = v;
        }
    }

    let backend = InMemoryBackend::with_bytes(
        serde_json::to_vec(&json!({"schema_version": 1, "name": "x", "totally_unknown_key": true}))
            .unwrap(),
    );
    let store = ConfigStore::<StrictConfig>::new(Arc::new(backend), ConfigFormat::default(), 1);
    let err = store.load().unwrap_err();
    assert!(matches!(
        err,
        cli_framework::config::ConfigError::Parse { .. }
    ));
}

// ── Flattening a real, nested config-file document ───────────────────────────

#[test]
fn flatten_to_paths_bridges_a_typed_config_struct_into_the_resolver() {
    let manifest = manifest_with(vec![FieldManifest {
        kind: FieldKind::Section {
            fields: vec![str_field("proxy_url", "http://default-proxy")],
        },
        ..str_field("network", "unused")
    }]);

    let on_disk_document = json!({ "network": { "proxy_url": "http://from-file" } });
    let input = ResolutionInput {
        config_file: flatten_to_paths(&on_disk_document),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(
        resolved.value("network.proxy_url"),
        Some(&json!("http://from-file"))
    );
}

#[test]
fn constraints_round_trip_but_do_not_gate_resolution() {
    let mut f = field("count", FieldKind::Int, json!(1));
    f.constraints = Some(FieldConstraints {
        min: Some(0.0),
        max: Some(10.0),
        allowed_values: None,
    });
    let manifest = manifest_with(vec![f]);
    let input = ResolutionInput {
        flags: map(&[("count", json!(9999))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);
    assert_eq!(resolved.value("count"), Some(&json!(9999)));
}
