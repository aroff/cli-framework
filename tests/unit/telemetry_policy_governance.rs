// tests/unit/telemetry_policy_governance.rs
//
// Adapted from the plan's literal listing against the real crate surface
// (confirmed by reading src/config/service/mod.rs, src/config/resolution/
// resolver.rs and src/config/service/types.rs before writing this):
//
// 1. `config::service::types` and `config::service::validate` are private
//    submodules (`mod types;` / `mod validate;` in service/mod.rs, not
//    `pub mod`) — `StoredPolicy` and `validate_stored_policy` are re-exported
//    at `config::service` directly instead. The plan's literal
//    `config::service::types::StoredPolicy` / `config::service::validate::
//    validate_stored_policy` paths fail to compile with E0603 ("module is
//    private"); confirmed by running the plan's exact text first.
// 2. `StoredPolicy` does not derive `Default` (by design, per its own doc
//    comment history) — constructed here via a local `stored_policy` helper
//    that names every field, rather than `..Default::default()`.
// 3. `Resolved` exposes its per-path data via the methods `value(path)` /
//    `provenance(path)` (see resolver.rs), not an `entries` field — `entries`
//    is itself a method returning `Vec<ResolvedEntry>` for "the whole
//    resolved surface at once", not a keyed lookup. The plan's
//    `resolved.entries.get(path)` fails to compile with E0615 ("method, not a
//    field"). Adapted to the real per-path accessors, which is both what
//    compiles and the more direct way to ask "what is this one field".
use cli_framework::config::resolution::{resolve, Layer, ResolutionInput, WarningReason};
use cli_framework::config::service::{validate_stored_policy, StoredPolicy};
use cli_framework::config::StaleAction;
use cli_framework::telemetry::{telemetry_only_manifest, ProbeRegistry};
use serde_json::{json, Map, Value};

fn manifest() -> cli_framework::config::manifest::ConfigManifest {
    telemetry_only_manifest("demo", &ProbeRegistry::with_builtins(), None)
}

fn tree(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

/// `StoredPolicy` names every field explicitly (it does not derive
/// `Default`) — this centralizes the boilerplate the plan's literal
/// `..Default::default()` would otherwise have supplied.
fn stored_policy(enforced: Map<String, Value>, recommended: Map<String, Value>) -> StoredPolicy {
    StoredPolicy {
        app: "demo".to_string(),
        profile: "default".to_string(),
        enforced,
        recommended,
        parent_profile: None,
        max_cache_age_secs: 3600,
        stale_action: StaleAction::Warn,
        version: 1,
    }
}

#[test]
fn the_config_service_refuses_to_store_an_enforced_telemetry_level() {
    let policy = stored_policy(tree(&[("telemetry.level", json!("debug"))]), Map::new());
    let errors = validate_stored_policy(&manifest(), &policy);
    assert_eq!(
        errors.len(),
        1,
        "expected exactly one violation, got {errors:?}"
    );
    let rendered = errors[0].to_string();
    assert!(
        rendered.contains("telemetry.level"),
        "the error must name the field: {rendered}"
    );
}

#[test]
fn the_config_service_accepts_a_recommended_telemetry_level() {
    let policy = stored_policy(Map::new(), tree(&[("telemetry.level", json!("usage"))]));
    assert!(validate_stored_policy(&manifest(), &policy).is_empty());
}

#[test]
fn a_client_that_receives_an_enforced_level_anyway_drops_it_and_warns() {
    let input = ResolutionInput {
        enforced: tree(&[("telemetry.level", json!("debug"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest(), &input);
    assert_eq!(
        resolved.provenance("telemetry.level").unwrap().layer,
        Layer::Default,
        "the enforced value must not have been applied"
    );
    assert!(resolved.warnings.iter().any(
        |w| w.path == "telemetry.level" && w.reason == WarningReason::NotEnforceableInEnforced
    ));
}

#[test]
fn an_organisation_may_enforce_the_endpoint_and_a_probe_switch() {
    let policy = stored_policy(
        tree(&[
            ("telemetry.endpoint", json!("http://collector:4318")),
            ("telemetry.cli.command.args.enabled", json!(false)),
        ]),
        Map::new(),
    );
    assert!(validate_stored_policy(&manifest(), &policy).is_empty());
}

#[test]
fn an_organisation_may_not_set_the_install_id_at_all() {
    let policy = stored_policy(
        Map::new(),
        tree(&[("telemetry.install_id", json!("forced"))]),
    );
    assert!(
        !validate_stored_policy(&manifest(), &policy).is_empty(),
        "install_id is local_only and not manageable"
    );
}

#[test]
fn an_enforced_endpoint_actually_reaches_the_resolved_value() {
    let input = ResolutionInput {
        config_file: tree(&[("telemetry.endpoint", json!("http://local:4318"))]),
        enforced: tree(&[("telemetry.endpoint", json!("http://fleet:4318"))]),
        ..Default::default()
    };
    let resolved = resolve(&manifest(), &input);
    assert_eq!(
        resolved.value("telemetry.endpoint"),
        Some(&json!("http://fleet:4318"))
    );
    let provenance = resolved.provenance("telemetry.endpoint").unwrap();
    assert_eq!(provenance.layer, Layer::Enforced);
    assert!(provenance.locked);
}
