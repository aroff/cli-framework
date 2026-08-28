//! Startup manifest-conformance validation (spec 022, "Validation at
//! startup"): one test per rejection reason, plus the negative check that
//! the *same* field, made conforming, validates cleanly — proving each
//! assertion actually discriminates rather than always rejecting.
//!
//! `validate_stored_policy` calls `crate::config::resolution`'s own
//! `server_tree_drop_reason_recommended`/`server_tree_drop_reason_enforced`
//! (made `pub(crate)` specifically for this) rather than a second hand-rolled
//! copy of the rule set — these tests are pinning that shared rule set
//! through the config-service entry point, not a parallel implementation
//! that could silently drift from it.

use cli_framework::config::manifest::{
    ConfigManifest, FieldConstraints, FieldKind, FieldManifest, Scope,
};
use cli_framework::config::service::{validate_stored_policy, PolicyValidationError, StoredPolicy};
use cli_framework::config::StaleAction;
use serde_json::{json, Map};

fn field(key: &str, kind: FieldKind) -> FieldManifest {
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

fn manifest(fields: Vec<FieldManifest>) -> ConfigManifest {
    ConfigManifest::new("myapp", fields)
}

fn policy_with(
    enforced: Map<String, serde_json::Value>,
    recommended: Map<String, serde_json::Value>,
) -> StoredPolicy {
    StoredPolicy {
        app: "myapp".to_string(),
        profile: "developers".to_string(),
        enforced,
        recommended,
        parent_profile: None,
        max_cache_age_secs: 3600,
        stale_action: StaleAction::Warn,
        version: 1,
    }
}

fn only_enforced(key: &str, value: serde_json::Value) -> StoredPolicy {
    let mut enforced = Map::new();
    enforced.insert(key.to_string(), value);
    policy_with(enforced, Map::new())
}

fn only_recommended(key: &str, value: serde_json::Value) -> StoredPolicy {
    let mut recommended = Map::new();
    recommended.insert(key.to_string(), value);
    policy_with(Map::new(), recommended)
}

#[test]
fn unknown_field_is_rejected() {
    let m = manifest(vec![field("greeting", FieldKind::Str)]);
    let p = only_enforced("does_not_exist", json!("x"));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.iter().any(|e| matches!(e, PolicyValidationError::UnknownField { path, .. } if path == "does_not_exist")),
        "expected UnknownField, got {errors:?}"
    );
}

#[test]
fn unknown_field_negative_check_a_declared_field_does_not_trip_this_rule() {
    let m = manifest(vec![field("greeting", FieldKind::Str)]);
    let p = only_enforced("greeting", json!("hello"));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.is_empty(),
        "declared field must validate cleanly, got {errors:?}"
    );
}

#[test]
fn type_mismatch_is_rejected() {
    let m = manifest(vec![field("count", FieldKind::Int)]);
    let p = only_enforced("count", json!("not-a-number"));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.iter().any(
            |e| matches!(e, PolicyValidationError::TypeMismatch { path, .. } if path == "count")
        ),
        "expected TypeMismatch, got {errors:?}"
    );
}

#[test]
fn type_mismatch_negative_check_the_correct_type_validates_cleanly() {
    let m = manifest(vec![field("count", FieldKind::Int)]);
    let p = only_enforced("count", json!(42));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.is_empty(),
        "correctly-typed value must validate cleanly, got {errors:?}"
    );
}

#[test]
fn secret_field_is_rejected() {
    let mut f = field("api_key", FieldKind::Str);
    f.secret = true;
    let m = manifest(vec![f]);
    let p = only_enforced("api_key", json!("shh"));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, PolicyValidationError::Secret { path, .. } if path == "api_key")),
        "expected Secret, got {errors:?}"
    );
}

#[test]
fn secret_field_negative_check_a_non_secret_field_of_the_same_shape_validates_cleanly() {
    let f = field("api_key", FieldKind::Str);
    let m = manifest(vec![f]);
    let p = only_enforced("api_key", json!("not actually secret in this manifest"));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.is_empty(),
        "non-secret field must validate cleanly, got {errors:?}"
    );
}

#[test]
fn local_only_field_is_rejected() {
    let mut f = field("service_address", FieldKind::Url);
    f.local_only = true;
    let m = manifest(vec![f]);
    let p = only_enforced("service_address", json!("https://evil.example.com"));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.iter().any(|e| matches!(e, PolicyValidationError::LocalOnly { path, .. } if path == "service_address")),
        "expected LocalOnly, got {errors:?}"
    );
}

#[test]
fn local_only_field_negative_check_an_ordinary_field_validates_cleanly() {
    let f = field("service_address", FieldKind::Url);
    let m = manifest(vec![f]);
    let p = only_enforced("service_address", json!("https://fine.example.com"));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.is_empty(),
        "non-local_only field must validate cleanly, got {errors:?}"
    );
}

#[test]
fn not_manageable_field_is_rejected() {
    let mut f = field("license_seat_count", FieldKind::Int);
    f.manageable = false;
    let m = manifest(vec![f]);
    let p = only_recommended("license_seat_count", json!(5));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.iter().any(|e| matches!(e, PolicyValidationError::NotManageable { path, .. } if path == "license_seat_count")),
        "expected NotManageable, got {errors:?}"
    );
}

#[test]
fn not_manageable_field_negative_check_a_manageable_field_validates_cleanly() {
    let f = field("license_seat_count", FieldKind::Int);
    let m = manifest(vec![f]);
    let p = only_recommended("license_seat_count", json!(5));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.is_empty(),
        "manageable field must validate cleanly, got {errors:?}"
    );
}

#[test]
fn org_scope_in_recommended_is_rejected() {
    let mut f = field("compliance_mode", FieldKind::Bool);
    f.scope = Scope::Org;
    let m = manifest(vec![f]);
    let p = only_recommended("compliance_mode", json!(true));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.iter().any(|e| matches!(e, PolicyValidationError::OrgScopeInRecommended { path, .. } if path == "compliance_mode")),
        "expected OrgScopeInRecommended, got {errors:?}"
    );
}

#[test]
fn org_scope_in_enforced_is_valid_the_rule_is_specific_to_recommended() {
    let mut f = field("compliance_mode", FieldKind::Bool);
    f.scope = Scope::Org;
    let m = manifest(vec![f]);
    let p = only_enforced("compliance_mode", json!(true));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.is_empty(),
        "org-scoped field in enforced is the *only* valid place for it, got {errors:?}"
    );
}

#[test]
fn enforceable_false_field_in_enforced_is_rejected() {
    let mut f = field("telemetry_opt_in", FieldKind::Bool);
    f.enforceable = false;
    let m = manifest(vec![f]);
    let p = only_enforced("telemetry_opt_in", json!(true));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.iter().any(|e| matches!(e, PolicyValidationError::NotEnforceable { path, .. } if path == "telemetry_opt_in")),
        "expected NotEnforceable, got {errors:?}"
    );
}

#[test]
fn enforceable_false_field_in_recommended_is_valid_the_rule_is_specific_to_enforced() {
    let mut f = field("telemetry_opt_in", FieldKind::Bool);
    f.enforceable = false;
    let m = manifest(vec![f]);
    let p = only_recommended("telemetry_opt_in", json!(true));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.is_empty(),
        "enforceable=false fields may still be recommended, got {errors:?}"
    );
}

#[test]
fn every_error_reports_the_app_and_profile_it_came_from() {
    let m = manifest(vec![field("greeting", FieldKind::Str)]);
    let p = only_enforced("ghost", json!(1));
    let errors = validate_stored_policy(&m, &p);
    match &errors[0] {
        PolicyValidationError::UnknownField { app, profile, .. } => {
            assert_eq!(app, "myapp");
            assert_eq!(profile, "developers");
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
}

#[test]
fn every_violation_across_both_trees_is_reported_in_one_pass_not_just_the_first() {
    let m = manifest(vec![field("greeting", FieldKind::Str)]);
    let mut enforced = Map::new();
    enforced.insert("unknown_one".to_string(), json!(1));
    enforced.insert("unknown_two".to_string(), json!(2));
    let mut recommended = Map::new();
    recommended.insert("unknown_three".to_string(), json!(3));
    let p = policy_with(enforced, recommended);
    let errors = validate_stored_policy(&m, &p);
    assert_eq!(
        errors.len(),
        3,
        "expected all three violations, got {errors:?}"
    );
}

#[test]
fn constraints_do_not_affect_validation_they_are_advisory_only() {
    let mut f = field("count", FieldKind::Int);
    f.constraints = Some(FieldConstraints {
        min: Some(0.0),
        max: Some(10.0),
        allowed_values: None,
    });
    let m = manifest(vec![f]);
    let p = only_enforced("count", json!(9999));
    let errors = validate_stored_policy(&m, &p);
    assert!(
        errors.is_empty(),
        "constraints are not enforced by this validator, got {errors:?}"
    );
}
