//! Startup manifest-conformance validation (spec 022, "Validation at
//! startup"): every stored policy must be valid against its application's
//! manifest before the service will serve anything.
//!
//! This calls `crate::config::resolution`'s own drop-reason rules —
//! `server_tree_drop_reason_recommended`/`server_tree_drop_reason_enforced`,
//! made `pub(crate)` in that module specifically for this call site — rather
//! than re-implementing the unknown-field / type-mismatch / secret /
//! local_only / org-scope-must-be-enforced / enforceable-false-must-not-be-
//! enforced rule set a second time. The task's own instructions are explicit
//! that a second hand-rolled copy is exactly what must be avoided here, since
//! two copies of "what makes a policy value valid" could silently drift
//! apart — one accepting something the other would reject.

use super::error::PolicyValidationError;
use super::store::PolicyStore;
use super::types::StoredPolicy;
use crate::config::manifest::ConfigManifest;
use crate::config::resolution::{
    server_tree_drop_reason_enforced, server_tree_drop_reason_recommended, WarningReason,
};
use std::collections::HashMap;

/// Validate one stored policy's `enforced`/`recommended` trees against
/// `manifest`, returning every violation found (not just the first) so a
/// deployment operator sees the whole picture in one pass.
///
/// Does **not** check inheritance integrity — that spans every policy for
/// the application at once and is handled by [`validate_all`] via
/// `crate::config::service::inherit::resolve_chain`, not per-policy.
pub fn validate_stored_policy(
    manifest: &ConfigManifest,
    policy: &StoredPolicy,
) -> Vec<PolicyValidationError> {
    let mut errors = Vec::new();

    for (path, value) in &policy.recommended {
        match manifest.leaf_by_path(path) {
            None => errors.push(PolicyValidationError::UnknownField {
                app: policy.app.clone(),
                profile: policy.profile.clone(),
                path: path.clone(),
            }),
            Some(field) => {
                if let Some(reason) = server_tree_drop_reason_recommended(field, value) {
                    errors.push(map_reason(&policy.app, &policy.profile, path, reason));
                }
            }
        }
    }

    for (path, value) in &policy.enforced {
        match manifest.leaf_by_path(path) {
            None => errors.push(PolicyValidationError::UnknownField {
                app: policy.app.clone(),
                profile: policy.profile.clone(),
                path: path.clone(),
            }),
            Some(field) => {
                if let Some(reason) = server_tree_drop_reason_enforced(field, value) {
                    errors.push(map_reason(&policy.app, &policy.profile, path, reason));
                }
            }
        }
    }

    errors
}

fn map_reason(
    app: &str,
    profile: &str,
    path: &str,
    reason: WarningReason,
) -> PolicyValidationError {
    let app = app.to_string();
    let profile = profile.to_string();
    let path = path.to_string();
    match reason {
        WarningReason::LocalOnlyInServerTree => {
            PolicyValidationError::LocalOnly { app, profile, path }
        }
        WarningReason::NotManageableInServerTree => {
            PolicyValidationError::NotManageable { app, profile, path }
        }
        WarningReason::SecretInServerTree => PolicyValidationError::Secret { app, profile, path },
        WarningReason::OrgScopeInRecommended => {
            PolicyValidationError::OrgScopeInRecommended { app, profile, path }
        }
        WarningReason::NotEnforceableInEnforced => {
            PolicyValidationError::NotEnforceable { app, profile, path }
        }
        // `push_unknown_key_warnings` (a separate step inside `resolve()`,
        // not part of the two drop-reason functions this module calls) is
        // the only producer of `UnknownKey` in the resolver — the
        // drop-reason functions themselves never return it, since they're
        // only ever invoked (both here and in `resolve()`) after a
        // successful `manifest.leaf_by_path` lookup already proved the
        // field exists. Kept as an explicit arm (mapping to the same
        // `UnknownField` variant `leaf_by_path`'s `None` branch above
        // produces) rather than `unreachable!()`, purely so this `match`
        // stays exhaustive if `WarningReason` ever gains a variant that
        // legitimately can flow through here.
        WarningReason::UnknownKey => PolicyValidationError::UnknownField { app, profile, path },
        WarningReason::TypeMismatch => PolicyValidationError::TypeMismatch { app, profile, path },
    }
}

/// Validate every stored policy for every application `store` knows about,
/// including inheritance integrity (every `parent_profile` resolves, no
/// cycles). Returns every error found across the whole store — see
/// [`super::error::StartupValidationError`].
///
/// Intended to be called once, by the embedding application, before
/// [`super::router::config_service_router`] starts accepting traffic (spec
/// 022 user story 27: "storage validated at startup... a broken policy set
/// fails deployment rather than surfacing per request").
pub async fn validate_all(
    store: &dyn PolicyStore,
) -> Result<(), super::error::StartupValidationError> {
    let mut errors = Vec::new();

    let apps = match store.apps().await {
        Ok(apps) => apps,
        Err(e) => {
            return Err(super::error::StartupValidationError(vec![
                PolicyValidationError::UnknownField {
                    app: "*".to_string(),
                    profile: "*".to_string(),
                    path: format!("<storage error listing apps: {e}>"),
                },
            ]))
        }
    };

    for app in &apps {
        let manifest = match store.manifest(app).await {
            Ok(Some(m)) => Some(m.doc),
            Ok(None) => None,
            Err(e) => {
                errors.push(PolicyValidationError::UnknownField {
                    app: app.clone(),
                    profile: "*".to_string(),
                    path: format!("<storage error loading manifest: {e}>"),
                });
                None
            }
        };

        let policies = match store.policies_for_app(app).await {
            Ok(p) => p,
            Err(e) => {
                errors.push(PolicyValidationError::UnknownField {
                    app: app.clone(),
                    profile: "*".to_string(),
                    path: format!("<storage error loading policies: {e}>"),
                });
                continue;
            }
        };

        let by_profile: HashMap<&str, &StoredPolicy> =
            policies.iter().map(|p| (p.profile.as_str(), p)).collect();

        // Bug 2 / Bug 4: `validate_all` previously never called
        // `assignment_rules` at all, so (a) a row with an unparseable
        // operator (a `StoreError` from the backend's own parsing, e.g.
        // `PgPolicyStore::assignment_rules`) never surfaced until a real
        // request hit it, and (b) nothing ever checked that a rule's (or
        // the terminal default's) target `profile` actually exists for
        // this app. Both are checked unconditionally, independent of
        // whether a manifest exists — an assignment row targets a profile,
        // not a manifest field.
        match store.assignment_rules(app).await {
            Ok(rules) => {
                for rule in &rules {
                    if !by_profile.contains_key(rule.profile.as_str()) {
                        errors.push(PolicyValidationError::AssignmentRuleMissingProfile {
                            app: app.clone(),
                            ord: rule.ord,
                            profile: rule.profile.clone(),
                        });
                    }
                }

                // Bug 4: a `Default`-operator row (see
                // `super::types::RuleOperator::Default`'s doc comment) must
                // be the last-ordered row for its app, or it silently
                // preempts every rule ordered after it.
                if let Some(max_ord) = rules.iter().map(|r| r.ord).max() {
                    for rule in &rules {
                        if rule.operator == super::types::RuleOperator::Default
                            && rule.ord != max_ord
                        {
                            errors.push(PolicyValidationError::DefaultRuleNotLast {
                                app: app.clone(),
                                ord: rule.ord,
                                max_ord,
                            });
                        }
                    }
                }
            }
            Err(e) => {
                errors.push(PolicyValidationError::UnknownField {
                    app: app.clone(),
                    profile: "*".to_string(),
                    path: format!("<storage error loading assignment rules: {e}>"),
                });
            }
        }

        let Some(manifest) = manifest else {
            for policy in &policies {
                errors.push(PolicyValidationError::MissingManifest {
                    app: app.clone(),
                    profile: policy.profile.clone(),
                });
            }
            continue;
        };

        for policy in &policies {
            errors.extend(validate_stored_policy(&manifest, policy));
        }

        for policy in &policies {
            if let Err(e) = super::inherit::resolve_chain(&by_profile, &policy.profile) {
                errors.push(match e {
                    super::error::InheritanceError::ProfileNotFound { .. } => {
                        // Unreachable here: `policy.profile` is itself a key
                        // of `by_profile` by construction.
                        continue;
                    }
                    super::error::InheritanceError::MissingParent { child, parent } => {
                        PolicyValidationError::MissingParent {
                            app: app.clone(),
                            profile: child,
                            parent,
                        }
                    }
                    super::error::InheritanceError::Cycle { profile } => {
                        PolicyValidationError::InheritanceCycle {
                            app: app.clone(),
                            profile,
                        }
                    }
                });
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(super::error::StartupValidationError(errors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{FieldKind, FieldManifest, Scope};
    use crate::config::service::error::StoreError;
    use crate::config::service::types::{AssignmentRule, RuleOperator, StoredManifest};
    use crate::config::StaleAction;
    use async_trait::async_trait;
    use serde_json::{json, Map};

    fn empty_manifest(app: &str) -> StoredManifest {
        StoredManifest {
            app: app.to_string(),
            doc: ConfigManifest::new(app, vec![]),
            version: 1,
        }
    }

    fn policy(app: &str, profile: &str, parent: Option<&str>) -> StoredPolicy {
        StoredPolicy {
            app: app.to_string(),
            profile: profile.to_string(),
            enforced: Map::new(),
            recommended: Map::new(),
            parent_profile: parent.map(str::to_string),
            max_cache_age_secs: 3600,
            stale_action: StaleAction::Warn,
            version: 1,
        }
    }

    /// A `PolicyStore` whose every method is independently overridable via
    /// `Result`s baked in at construction time — used to exercise
    /// `validate_all`'s storage-error branches, which a real backend can't
    /// be coaxed into failing on demand.
    struct ScriptedStore {
        apps: Result<Vec<String>, &'static str>,
        manifest: Result<Option<StoredManifest>, &'static str>,
        policies: Result<Vec<StoredPolicy>, &'static str>,
        assignment_rules: Result<Vec<AssignmentRule>, &'static str>,
    }

    impl ScriptedStore {
        /// Convenience constructor for tests that don't care about
        /// assignment rules at all — equivalent to the pre-Bug-2 behavior
        /// where `assignment_rules` always answered `Ok(vec![])`.
        fn new(
            apps: Result<Vec<String>, &'static str>,
            manifest: Result<Option<StoredManifest>, &'static str>,
            policies: Result<Vec<StoredPolicy>, &'static str>,
        ) -> Self {
            Self {
                apps,
                manifest,
                policies,
                assignment_rules: Ok(vec![]),
            }
        }
    }

    #[async_trait]
    impl PolicyStore for ScriptedStore {
        async fn manifest(&self, _app: &str) -> Result<Option<StoredManifest>, StoreError> {
            self.manifest.clone().map_err(StoreError::backend)
        }
        async fn policy(
            &self,
            _app: &str,
            _profile: &str,
        ) -> Result<Option<StoredPolicy>, StoreError> {
            unreachable!("validate_all never calls policy() directly")
        }
        async fn policies_for_app(&self, _app: &str) -> Result<Vec<StoredPolicy>, StoreError> {
            self.policies.clone().map_err(StoreError::backend)
        }
        async fn assignment_rules(&self, _app: &str) -> Result<Vec<AssignmentRule>, StoreError> {
            self.assignment_rules.clone().map_err(StoreError::backend)
        }
        async fn apps(&self) -> Result<Vec<String>, StoreError> {
            self.apps.clone().map_err(StoreError::backend)
        }
    }

    fn rule(app: &str, ord: i64, operator: RuleOperator, profile: &str) -> AssignmentRule {
        AssignmentRule {
            app: app.to_string(),
            ord,
            claim_path: "team".to_string(),
            operator,
            value: Some(json!("x")),
            profile: profile.to_string(),
        }
    }

    #[tokio::test]
    async fn apps_listing_failure_is_reported_and_stops_validation() {
        let store = ScriptedStore::new(Err("apps table unreachable"), Ok(None), Ok(vec![]));
        let err = validate_all(&store).await.unwrap_err();
        assert_eq!(err.0.len(), 1);
        assert!(format!("{}", err.0[0]).contains("apps table unreachable"));
    }

    #[tokio::test]
    async fn manifest_lookup_failure_is_reported_per_app() {
        let store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Err("manifest table unreachable"),
            Ok(vec![]),
        );
        let err = validate_all(&store).await.unwrap_err();
        assert_eq!(err.0.len(), 1);
        assert!(format!("{}", err.0[0]).contains("manifest table unreachable"));
    }

    #[tokio::test]
    async fn policies_lookup_failure_is_reported_and_skips_the_rest_of_that_app() {
        let store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(Some(empty_manifest("myapp"))),
            Err("policy table unreachable"),
        );
        let err = validate_all(&store).await.unwrap_err();
        assert_eq!(err.0.len(), 1);
        assert!(format!("{}", err.0[0]).contains("policy table unreachable"));
    }

    #[tokio::test]
    async fn a_policy_with_no_manifest_at_all_is_missing_manifest_for_every_profile() {
        let store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(None),
            Ok(vec![
                policy("myapp", "base", None),
                policy("myapp", "kiosk", None),
            ]),
        );
        let err = validate_all(&store).await.unwrap_err();
        assert_eq!(err.0.len(), 2);
        assert!(err
            .0
            .iter()
            .all(|e| matches!(e, PolicyValidationError::MissingManifest { .. })));
    }

    #[tokio::test]
    async fn a_policy_naming_a_parent_with_no_stored_policy_is_missing_parent() {
        let store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(Some(empty_manifest("myapp"))),
            Ok(vec![policy("myapp", "child", Some("ghost-parent"))]),
        );
        let err = validate_all(&store).await.unwrap_err();
        assert!(err
            .0
            .iter()
            .any(|e| matches!(e, PolicyValidationError::MissingParent { parent, .. } if parent == "ghost-parent")));
    }

    #[tokio::test]
    async fn a_two_profile_inheritance_cycle_is_reported() {
        let store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(Some(empty_manifest("myapp"))),
            Ok(vec![
                policy("myapp", "a", Some("b")),
                policy("myapp", "b", Some("a")),
            ]),
        );
        let err = validate_all(&store).await.unwrap_err();
        assert!(err
            .0
            .iter()
            .any(|e| matches!(e, PolicyValidationError::InheritanceCycle { .. })));
    }

    #[tokio::test]
    async fn a_fully_valid_multi_app_store_validates_cleanly() {
        let f = FieldManifest {
            key: "greeting".to_string(),
            kind: FieldKind::Str,
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
        };
        let manifest = StoredManifest {
            app: "myapp".to_string(),
            doc: ConfigManifest::new("myapp", vec![f]),
            version: 1,
        };
        let mut p = policy("myapp", "base", None);
        p.enforced.insert("greeting".to_string(), json!("hi"));

        let store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(Some(manifest)),
            Ok(vec![p]),
        );
        assert!(validate_all(&store).await.is_ok());
    }

    #[tokio::test]
    async fn a_store_with_no_apps_at_all_validates_cleanly() {
        let store = ScriptedStore::new(Ok(vec![]), Ok(None), Ok(vec![]));
        assert!(validate_all(&store).await.is_ok());
    }

    // ── Bug 2: assignment rules were never validated at startup at all ────

    #[tokio::test]
    async fn assignment_rules_storage_failure_is_reported_not_silently_ignored() {
        // Before the fix, `validate_all` never called `assignment_rules` at
        // all -- a backend error here (e.g. an unparseable operator, which
        // `PgPolicyStore::assignment_rules` surfaces as a `StoreError`)
        // would only be discovered later, at request time.
        let mut store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(Some(empty_manifest("myapp"))),
            Ok(vec![]),
        );
        store.assignment_rules = Err("assignment table has an unparseable operator");
        let err = validate_all(&store).await.unwrap_err();
        assert!(
            err.0
                .iter()
                .any(|e| format!("{e}").contains("assignment table has an unparseable operator")),
            "got {:?}",
            err.0
        );
    }

    #[tokio::test]
    async fn an_assignment_rule_targeting_a_nonexistent_profile_is_rejected() {
        let mut store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(Some(empty_manifest("myapp"))),
            Ok(vec![policy("myapp", "base", None)]),
        );
        store.assignment_rules = Ok(vec![rule(
            "myapp",
            0,
            RuleOperator::Equals,
            "ghost-profile",
        )]);
        let err = validate_all(&store).await.unwrap_err();
        assert!(
            err.0.iter().any(|e| matches!(
                e,
                PolicyValidationError::AssignmentRuleMissingProfile { profile, ord: 0, .. }
                    if profile == "ghost-profile"
            )),
            "expected AssignmentRuleMissingProfile, got {:?}",
            err.0
        );
    }

    #[tokio::test]
    async fn an_assignment_rule_targeting_an_existing_profile_is_the_negative_check() {
        let mut store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(Some(empty_manifest("myapp"))),
            Ok(vec![policy("myapp", "base", None)]),
        );
        store.assignment_rules = Ok(vec![rule("myapp", 0, RuleOperator::Equals, "base")]);
        assert!(
            validate_all(&store).await.is_ok(),
            "a rule targeting a real profile must validate cleanly"
        );
    }

    // ── Bug 4: a `Default` row must be the last-ordered assignment rule ───

    #[tokio::test]
    async fn a_default_rule_that_is_not_last_ordered_is_rejected() {
        let mut store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(Some(empty_manifest("myapp"))),
            Ok(vec![
                policy("myapp", "base", None),
                policy("myapp", "fallback", None),
            ]),
        );
        // The default (ord 0) is ordered *before* a real rule (ord 1) --
        // it would silently preempt that rule every time (bug 4).
        store.assignment_rules = Ok(vec![
            AssignmentRule {
                app: "myapp".to_string(),
                ord: 0,
                claim_path: String::new(),
                operator: RuleOperator::Default,
                value: None,
                profile: "fallback".to_string(),
            },
            rule("myapp", 1, RuleOperator::Equals, "base"),
        ]);
        let err = validate_all(&store).await.unwrap_err();
        assert!(
            err.0.iter().any(|e| matches!(
                e,
                PolicyValidationError::DefaultRuleNotLast {
                    ord: 0,
                    max_ord: 1,
                    ..
                }
            )),
            "expected DefaultRuleNotLast, got {:?}",
            err.0
        );
    }

    #[tokio::test]
    async fn a_default_rule_that_is_last_ordered_is_the_negative_check() {
        let mut store = ScriptedStore::new(
            Ok(vec!["myapp".to_string()]),
            Ok(Some(empty_manifest("myapp"))),
            Ok(vec![
                policy("myapp", "base", None),
                policy("myapp", "fallback", None),
            ]),
        );
        store.assignment_rules = Ok(vec![
            rule("myapp", 0, RuleOperator::Equals, "base"),
            AssignmentRule {
                app: "myapp".to_string(),
                ord: 1,
                claim_path: String::new(),
                operator: RuleOperator::Default,
                value: None,
                profile: "fallback".to_string(),
            },
        ]);
        assert!(
            validate_all(&store).await.is_ok(),
            "a correctly-last default rule must validate cleanly"
        );
    }

    #[test]
    fn map_reason_unknown_key_arm_is_exhaustive_even_though_unreachable_in_practice() {
        // `push_unknown_key_warnings` (the resolver's actual producer of
        // `UnknownKey`) is never on the call path into `map_reason` -- see
        // the doc comment on that match arm -- but the arm still needs to
        // produce a sane value if `WarningReason` ever grows a case that
        // does reach it, so it's exercised directly here rather than left
        // as dead code no test ever proves compiles to something sane.
        let result = map_reason("app", "profile", "path", WarningReason::UnknownKey);
        assert!(matches!(result, PolicyValidationError::UnknownField { .. }));
    }
}
