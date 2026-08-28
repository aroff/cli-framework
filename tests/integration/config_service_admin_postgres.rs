//! `PolicyAdminStore` (spec 023) exercised directly against a real Postgres
//! — the storage-trait seam, same `DATABASE_URL`-presence skip contract as
//! `config_service_postgres_conformance.rs` (see that file's module docs for
//! the full rationale: CI always sets `DATABASE_URL` via the `postgres`
//! service container; local dev usually doesn't, which is a graceful skip,
//! not a failure).
//!
//! Every test claims a globally-unique app name (`uuid::Uuid::new_v4()`)
//! rather than truncating shared tables, for the same reason the
//! conformance suite does: `#[tokio::test]` functions in one binary run
//! concurrently by default, and this suite shares a long-lived Postgres
//! instance with everything else.

use cli_framework::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
use cli_framework::config::service::postgres::{connect_and_migrate, PgPolicyStore, PgPool};
use cli_framework::config::service::{
    AdminWriteError, AssignmentRule, FsPolicyStore, MutationKind, PolicyAdminStore, PolicyStore,
    PolicyValidationError, PolicyWrite, RuleOperator, StoredManifest, StoredPolicy,
};
use cli_framework::config::StaleAction;
use serde_json::{json, Map, Value};
use sqlx_core::row::Row;
use sqlx_core::types::Json;
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

async fn pool_or_skip() -> Option<PgPool> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping config-service admin Postgres suite: DATABASE_URL is not set. \
             CI always sets this; local dev usually doesn't, which is expected."
        );
        return None;
    };
    Some(
        connect_and_migrate(&url)
            .await
            .expect("connect + migrate against DATABASE_URL"),
    )
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

fn field(key: &str) -> FieldManifest {
    FieldManifest {
        key: key.to_string(),
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
    }
}

fn sample_manifest(app: &str) -> ConfigManifest {
    ConfigManifest::new(app, vec![field("greeting")])
}

fn empty_policy_write() -> PolicyWrite {
    PolicyWrite {
        enforced: Map::new(),
        recommended: Map::new(),
        parent_profile: None,
        max_cache_age_secs: 3600,
        stale_action: StaleAction::Warn,
    }
}

fn policy_write_with_greeting(value: &str) -> PolicyWrite {
    let mut enforced = Map::new();
    enforced.insert("greeting".to_string(), json!(value));
    PolicyWrite {
        enforced,
        recommended: Map::new(),
        parent_profile: None,
        max_cache_age_secs: 3600,
        stale_action: StaleAction::Warn,
    }
}

async fn count_mutation_log_rows(pool: &PgPool, app: &str, profile: Option<&str>) -> i64 {
    let row = match profile {
        Some(p) => sqlx_core::query::query::<sqlx_postgres::Postgres>(
            "SELECT COUNT(*) AS n FROM mutation_log WHERE app = $1 AND profile = $2",
        )
        .bind(app)
        .bind(p)
        .fetch_one(pool)
        .await
        .unwrap(),
        None => sqlx_core::query::query::<sqlx_postgres::Postgres>(
            "SELECT COUNT(*) AS n FROM mutation_log WHERE app = $1 AND profile IS NULL",
        )
        .bind(app)
        .fetch_one(pool)
        .await
        .unwrap(),
    };
    row.try_get::<i64, _>("n").unwrap()
}

// ── put_manifest ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn put_manifest_creates_a_new_manifest_at_version_one_with_one_log_row() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-manifest");
    let store = PgPolicyStore::new(pool.clone());

    let v = store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();
    assert_eq!(v, 1);

    let stored = PolicyStore::manifest(&store, &app).await.unwrap().unwrap();
    assert_eq!(stored.version, 1);
    assert_eq!(stored.doc.app, app);

    assert_eq!(count_mutation_log_rows(&pool, &app, None).await, 1);
}

#[tokio::test]
async fn put_manifest_stale_expected_version_is_a_conflict_and_changes_nothing() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-manifest-conflict");
    let store = PgPolicyStore::new(pool.clone());

    store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();

    let err = store
        .put_manifest(&app, sample_manifest(&app), "bob", 0)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            AdminWriteError::Conflict {
                current: 1,
                expected: 0
            }
        ),
        "got {err:?}"
    );

    // Rejected write appends zero log rows.
    assert_eq!(count_mutation_log_rows(&pool, &app, None).await, 1);
    let stored = PolicyStore::manifest(&store, &app).await.unwrap().unwrap();
    assert_eq!(
        stored.version, 1,
        "the conflicting write must not have landed"
    );
}

#[tokio::test]
async fn put_manifest_second_write_with_correct_expected_version_advances_the_version() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-manifest-update");
    let store = PgPolicyStore::new(pool.clone());

    store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();
    let v2 = store
        .put_manifest(
            &app,
            ConfigManifest::new(&app, vec![field("greeting"), field("farewell")]),
            "alice",
            1,
        )
        .await
        .unwrap();
    assert_eq!(v2, 2);

    let stored = PolicyStore::manifest(&store, &app).await.unwrap().unwrap();
    assert_eq!(stored.doc.iter_leaves().len(), 2);
    assert_eq!(count_mutation_log_rows(&pool, &app, None).await, 2);
}

// ── put_policy ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn put_policy_creates_a_new_profile_and_records_the_submitted_body() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-policy");
    let store = PgPolicyStore::new(pool.clone());
    let submitted = json!({"enforced": {"greeting": "hi"}});

    // Fix 2 (spec 023 review): `put_policy` now re-validates against a
    // locked manifest row inside its own transaction, rejecting with
    // `MissingManifest` if none exists for `app` -- a manifest must be
    // seeded first, unlike before this fix, when `PgPolicyStore::put_policy`
    // performed no manifest checking of its own at all.
    store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();

    let v = store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("hi"),
            MutationKind::PolicyPut,
            submitted.clone(),
            "alice",
            0,
        )
        .await
        .unwrap();
    assert_eq!(v, 1);

    let stored = PolicyStore::policy(&store, &app, "base")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.enforced.get("greeting"), Some(&json!("hi")));
    assert_eq!(stored.version, 1);

    let history = store.policy_history(&app, "base").await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, MutationKind::PolicyPut);
    assert_eq!(history[0].actor, "alice");
    assert_eq!(history[0].submitted, submitted);
    assert_eq!(history[0].resulting_version, 1);
    assert_eq!(
        history[0].resulting_document.get("enforced").unwrap()["greeting"],
        "hi"
    );
}

#[tokio::test]
async fn put_policy_stale_if_match_is_a_conflict_and_the_document_is_unchanged() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-policy-conflict");
    let store = PgPolicyStore::new(pool.clone());

    // Fix 2 (spec 023 review): see the matching comment in
    // `put_policy_creates_a_new_profile_and_records_the_submitted_body`.
    store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();

    store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("first"),
            MutationKind::PolicyPut,
            json!({}),
            "alice",
            0,
        )
        .await
        .unwrap();

    let err = store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("should-not-land"),
            MutationKind::PolicyPut,
            json!({}),
            "bob",
            0,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AdminWriteError::Conflict { .. }));

    let stored = PolicyStore::policy(&store, &app, "base")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.enforced.get("greeting"), Some(&json!("first")));
    assert_eq!(
        count_mutation_log_rows(&pool, &app, Some("base")).await,
        1,
        "a rejected write must append zero log rows"
    );
}

#[tokio::test]
async fn put_policy_can_be_reused_for_patch_and_restore_kinds() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-policy-kinds");
    let store = PgPolicyStore::new(pool.clone());

    // Fix 2 (spec 023 review): see the matching comment in
    // `put_policy_creates_a_new_profile_and_records_the_submitted_body`.
    store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();

    store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("v1"),
            MutationKind::PolicyPut,
            json!({"enforced": {"greeting": "v1"}}),
            "alice",
            0,
        )
        .await
        .unwrap();
    store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("v2"),
            MutationKind::PolicyPatch,
            json!({"enforced": {"greeting": "v2"}}),
            "alice",
            1,
        )
        .await
        .unwrap();
    store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("v1"),
            MutationKind::PolicyRestore,
            json!({"restore_from_version": 1}),
            "alice",
            2,
        )
        .await
        .unwrap();

    let history = store.policy_history(&app, "base").await.unwrap();
    assert_eq!(history.len(), 3);
    // History returns records in order with the versions they produced.
    assert_eq!(
        history
            .iter()
            .map(|h| h.resulting_version)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(history[0].kind, MutationKind::PolicyPut);
    assert_eq!(history[1].kind, MutationKind::PolicyPatch);
    assert_eq!(history[2].kind, MutationKind::PolicyRestore);

    // Restoring produced the earlier document (v1) as a NEW version (3),
    // with the intervening record (v2) still present and unchanged.
    let restored = PolicyStore::policy(&store, &app, "base")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.enforced.get("greeting"), Some(&json!("v1")));
    assert_eq!(restored.version, 3);
    assert_eq!(
        history[1].resulting_document.get("enforced").unwrap()["greeting"],
        "v2"
    );
}

/// User story 23: the mutation log survives deletion of the row it
/// describes. There is deliberately no HTTP delete endpoint in this PRD, so
/// this deletes the `policy` row directly via raw SQL — the concrete proof
/// that `mutation_log` has no FK relationship to `policy` (see
/// `002_admin_mutation_log.sql`'s own comment on why there is intentionally
/// no FK).
#[tokio::test]
async fn policy_history_survives_the_underlying_policy_row_being_deleted() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-policy-survives-delete");
    let store = PgPolicyStore::new(pool.clone());

    // Fix 2 (spec 023 review): see the matching comment in
    // `put_policy_creates_a_new_profile_and_records_the_submitted_body`.
    store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();

    store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("hi"),
            MutationKind::PolicyPut,
            json!({}),
            "alice",
            0,
        )
        .await
        .unwrap();

    sqlx_core::query::query::<sqlx_postgres::Postgres>(
        "DELETE FROM policy WHERE app = $1 AND profile = 'base'",
    )
    .bind(&app)
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        PolicyStore::policy(&store, &app, "base")
            .await
            .unwrap()
            .is_none(),
        "sanity check: the row is actually gone"
    );

    let history = store.policy_history(&app, "base").await.unwrap();
    assert_eq!(
        history.len(),
        1,
        "mutation_log must survive deletion of the policy row it describes"
    );
    assert_eq!(history[0].kind, MutationKind::PolicyPut);
}

// ── assignment rules ─────────────────────────────────────────────────────────

#[tokio::test]
async fn assignment_rules_version_is_zero_before_any_write() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-assignments-version");
    let store = PgPolicyStore::new(pool.clone());
    assert_eq!(store.assignment_rules_version(&app).await.unwrap(), 0);
}

#[tokio::test]
async fn put_assignment_rules_assigns_ord_from_array_position_not_the_callers_ord() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-assignments");
    let store = PgPolicyStore::new(pool.clone());

    // Deliberately wrong/out-of-order `ord` values on the input -- the store
    // must ignore them and assign strictly from array position.
    let rules = vec![
        AssignmentRule {
            app: app.clone(),
            ord: 99,
            claim_path: "team".to_string(),
            operator: RuleOperator::Equals,
            value: Some(json!("platform")),
            profile: "platform".to_string(),
        },
        AssignmentRule {
            app: app.clone(),
            ord: 1,
            claim_path: String::new(),
            operator: RuleOperator::Default,
            value: None,
            profile: "fallback".to_string(),
        },
    ];

    let v = store
        .put_assignment_rules(&app, rules, "alice", 0)
        .await
        .unwrap();
    assert_eq!(v, 1);
    assert_eq!(store.assignment_rules_version(&app).await.unwrap(), 1);

    let mut stored = PolicyStore::assignment_rules(&store, &app).await.unwrap();
    stored.sort_by_key(|r| r.ord);
    assert_eq!(stored[0].ord, 0);
    assert_eq!(stored[0].profile, "platform");
    assert_eq!(stored[1].ord, 1);
    assert_eq!(stored[1].profile, "fallback");

    assert_eq!(count_mutation_log_rows(&pool, &app, None).await, 1);
}

#[tokio::test]
async fn put_assignment_rules_replaces_the_whole_set() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-assignments-replace");
    let store = PgPolicyStore::new(pool.clone());

    let first = vec![AssignmentRule {
        app: app.clone(),
        ord: 0,
        claim_path: "team".to_string(),
        operator: RuleOperator::Equals,
        value: Some(json!("a")),
        profile: "a".to_string(),
    }];
    store
        .put_assignment_rules(&app, first, "alice", 0)
        .await
        .unwrap();

    let second = vec![AssignmentRule {
        app: app.clone(),
        ord: 0,
        claim_path: "team".to_string(),
        operator: RuleOperator::Equals,
        value: Some(json!("b")),
        profile: "b".to_string(),
    }];
    store
        .put_assignment_rules(&app, second, "alice", 1)
        .await
        .unwrap();

    let stored = PolicyStore::assignment_rules(&store, &app).await.unwrap();
    assert_eq!(
        stored.len(),
        1,
        "the old rule set must be fully replaced, not appended to"
    );
    assert_eq!(stored[0].profile, "b");
}

#[tokio::test]
async fn put_assignment_rules_stale_expected_version_is_a_conflict() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-assignments-conflict");
    let store = PgPolicyStore::new(pool.clone());

    store
        .put_assignment_rules(&app, vec![], "alice", 0)
        .await
        .unwrap();

    let err = store
        .put_assignment_rules(&app, vec![], "bob", 0)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        AdminWriteError::Conflict {
            current: 1,
            expected: 0
        }
    ));
}

// ── import_bundle ────────────────────────────────────────────────────────────

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[tokio::test]
async fn import_bundle_stores_manifest_policies_and_assignment_rules_in_one_go() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-import");
    let dir = tempfile::TempDir::new().unwrap();
    write(
        &dir.path().join(format!("manifests/{app}.json")),
        &serde_json::to_string(&sample_manifest(&app)).unwrap(),
    );
    write(
        &dir.path().join(format!("policies/{app}/base.toml")),
        r#"
        version = 1
        [enforced]
        "greeting" = "hi"
        "#,
    );
    write(
        &dir.path().join("assignments.toml"),
        &format!(
            r#"
            [{app}]
            default_profile = "base"
            "#
        ),
    );
    let bundle = FsPolicyStore::load(dir.path()).unwrap();

    let store = PgPolicyStore::new(pool.clone());
    let summary = store.import_bundle(&bundle, "alice").await.unwrap();
    assert_eq!(summary.manifests, 1);
    assert_eq!(summary.policies, 1);
    assert_eq!(summary.assignment_rules, 1);

    assert!(PolicyStore::manifest(&store, &app).await.unwrap().is_some());
    let policy = PolicyStore::policy(&store, &app, "base")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(policy.enforced.get("greeting"), Some(&json!("hi")));
    let rules = PolicyStore::assignment_rules(&store, &app).await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].operator, RuleOperator::Default);

    assert_eq!(
        count_mutation_log_rows(&pool, &app, None).await,
        1,
        "import appends exactly one mutation_log row per app"
    );
}

#[tokio::test]
async fn import_bundle_with_an_invalid_policy_stores_nothing_for_that_app() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-import-invalid");
    let dir = tempfile::TempDir::new().unwrap();
    write(
        &dir.path().join(format!("manifests/{app}.json")),
        &serde_json::to_string(&sample_manifest(&app)).unwrap(),
    );
    // References a field the manifest does not declare -- must fail
    // validation.
    write(
        &dir.path().join(format!("policies/{app}/base.toml")),
        r#"
        version = 1
        [enforced]
        "this_field_does_not_exist" = "x"
        "#,
    );
    let bundle = FsPolicyStore::load(dir.path()).unwrap();

    let store = PgPolicyStore::new(pool.clone());
    let err = store.import_bundle(&bundle, "alice").await.unwrap_err();
    assert!(matches!(err, AdminWriteError::Validation(_)), "got {err:?}");

    assert!(
        PolicyStore::manifest(&store, &app).await.unwrap().is_none(),
        "a bad import must store nothing, not even the manifest"
    );
    assert!(PolicyStore::policy(&store, &app, "base")
        .await
        .unwrap()
        .is_none());
    assert_eq!(count_mutation_log_rows(&pool, &app, None).await, 0);
}

/// Seeds an app with real prior state via the admin API itself, then
/// attempts a bad import of a *different* app in the same bundle. Proves
/// the seeded app's state is completely untouched -- the concrete
/// "read back the prior state" check spec 023's coverage list calls for,
/// not just "the bad app itself has nothing."
#[tokio::test]
async fn import_of_an_invalid_bundle_leaves_unrelated_prior_state_untouched() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let store = PgPolicyStore::new(pool.clone());

    let seeded_app = unique("admin-import-seeded");
    store
        .put_manifest(&seeded_app, sample_manifest(&seeded_app), "alice", 0)
        .await
        .unwrap();
    store
        .put_policy(
            &seeded_app,
            "base",
            policy_write_with_greeting("seeded"),
            MutationKind::PolicyPut,
            json!({}),
            "alice",
            0,
        )
        .await
        .unwrap();

    let broken_app = unique("admin-import-broken");
    let dir = tempfile::TempDir::new().unwrap();
    write(
        &dir.path().join(format!("manifests/{broken_app}.json")),
        &serde_json::to_string(&sample_manifest(&broken_app)).unwrap(),
    );
    write(
        &dir.path().join(format!("policies/{broken_app}/base.toml")),
        r#"
        version = 1
        [enforced]
        "ghost_field" = "x"
        "#,
    );
    let bundle = FsPolicyStore::load(dir.path()).unwrap();

    let err = store.import_bundle(&bundle, "alice").await.unwrap_err();
    assert!(matches!(err, AdminWriteError::Validation(_)));

    let seeded_policy = PolicyStore::policy(&store, &seeded_app, "base")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        seeded_policy.enforced.get("greeting"),
        Some(&json!("seeded")),
        "unrelated prior state must be untouched by a rejected import"
    );
    assert_eq!(seeded_policy.version, 1);
}

/// The genuine multi-app atomicity claim: a bundle naming TWO apps in one
/// import call, one valid and one invalid, must store NEITHER -- not just
/// "the invalid app itself has nothing" (the test above) but "a perfectly
/// valid app in the SAME bundle, submitted in the SAME call, is also
/// rejected." This is the one test whose passing genuinely depends on
/// "one transaction covering every write in the bundle" (spec 023) rather
/// than on validation alone -- see this file's own anti-vacuity exercise
/// (recorded in this slice's PR description) for the deliberate breakage
/// that confirms this.
#[tokio::test]
async fn import_rejects_the_whole_bundle_even_when_only_one_of_several_apps_is_invalid() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let store = PgPolicyStore::new(pool.clone());

    let valid_app = unique("admin-import-multi-valid");
    let broken_app = unique("admin-import-multi-broken");
    let dir = tempfile::TempDir::new().unwrap();
    write(
        &dir.path().join(format!("manifests/{valid_app}.json")),
        &serde_json::to_string(&sample_manifest(&valid_app)).unwrap(),
    );
    write(
        &dir.path().join(format!("policies/{valid_app}/base.toml")),
        r#"
        version = 1
        [enforced]
        "greeting" = "fine"
        "#,
    );
    write(
        &dir.path().join(format!("manifests/{broken_app}.json")),
        &serde_json::to_string(&sample_manifest(&broken_app)).unwrap(),
    );
    write(
        &dir.path().join(format!("policies/{broken_app}/base.toml")),
        r#"
        version = 1
        [enforced]
        "ghost_field" = "x"
        "#,
    );
    let bundle = FsPolicyStore::load(dir.path()).unwrap();

    let err = store.import_bundle(&bundle, "alice").await.unwrap_err();
    assert!(matches!(err, AdminWriteError::Validation(_)), "got {err:?}");

    assert!(
        PolicyStore::manifest(&store, &valid_app)
            .await
            .unwrap()
            .is_none(),
        "the VALID app in the same bundle must also not be stored -- the import is one atomic unit"
    );
    assert!(PolicyStore::policy(&store, &valid_app, "base")
        .await
        .unwrap()
        .is_none());
    assert_eq!(count_mutation_log_rows(&pool, &valid_app, None).await, 0);
}

/// Regression test: `import_bundle` must never touch `assignment`/
/// `assignment_set` for an app the bundle doesn't declare an
/// `assignments.toml` stanza for -- mirroring how a missing manifest or
/// policy file for that app is left untouched, not overwritten-to-empty.
/// Before the fix, `bundle.assignment_rules(app)` returning an empty `Vec`
/// (which it also does for a *declared-but-empty* stanza) was
/// indistinguishable from "not declared", so importing a bundle that only
/// carried a manifest for `app` silently deleted `app`'s real, pre-existing
/// assignment rules.
#[tokio::test]
async fn import_bundle_does_not_touch_assignments_for_an_app_that_declares_none() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let store = PgPolicyStore::new(pool.clone());

    let app = unique("admin-import-assignments-untouched");
    let seeded_rules = vec![AssignmentRule {
        app: app.clone(),
        ord: 0,
        claim_path: "team".to_string(),
        operator: RuleOperator::Equals,
        value: Some(json!("platform")),
        profile: "platform".to_string(),
    }];
    store
        .put_assignment_rules(&app, seeded_rules, "alice", 0)
        .await
        .unwrap();
    assert_eq!(store.assignment_rules_version(&app).await.unwrap(), 1);

    // A bundle that declares ONLY a manifest for `app` -- no `policies/`
    // entry, and critically no `assignments.toml` stanza at all.
    let dir = tempfile::TempDir::new().unwrap();
    write(
        &dir.path().join(format!("manifests/{app}.json")),
        &serde_json::to_string(&sample_manifest(&app)).unwrap(),
    );
    let bundle = FsPolicyStore::load(dir.path()).unwrap();

    let summary = store.import_bundle(&bundle, "alice").await.unwrap();
    assert_eq!(summary.assignment_rules, 0, "the bundle declared none");

    assert_eq!(
        store.assignment_rules_version(&app).await.unwrap(),
        1,
        "import must not bump/touch the assignment_set version for an app it didn't declare assignments for"
    );
    let rules = PolicyStore::assignment_rules(&store, &app).await.unwrap();
    assert_eq!(
        rules.len(),
        1,
        "the seeded assignment rule must survive an import that never mentioned assignments for this app"
    );
    assert_eq!(rules[0].profile, "platform");
}

// Export -> import round-tripping is exercised at the HTTP level, in
// `config_service_admin_router.rs` (`GET /v1/admin/export` followed by
// `POST /v1/admin/import`), since `build_export_tar`/`extract_bundle_from_tar`
// are `pub(crate)` to `config::service` and have no reason to be a public
// API surface of their own — the bundle format itself is already fully
// covered by `bundle.rs`'s own unit tests
// (`export_then_reload_via_fspolicystore_reproduces_the_same_documents`).

// ── PolicyWrite / MutationLogEntry / ImportSummary construction sanity ──────

#[test]
fn policy_write_and_stored_manifest_types_construct_as_expected() {
    // Cheap, non-DB sanity checks on the plain data types this suite builds
    // constantly above -- guards against a field being silently dropped from
    // a future refactor of `PolicyWrite`/`StoredManifest` without needing a
    // live database.
    let pw = empty_policy_write();
    assert!(pw.enforced.is_empty());
    assert_eq!(pw.max_cache_age_secs, 3600);
    assert_eq!(pw.stale_action, StaleAction::Warn);

    let sm = StoredManifest {
        app: "x".to_string(),
        doc: sample_manifest("x"),
        version: 1,
    };
    assert_eq!(sm.app, "x");

    let sp = StoredPolicy {
        app: "x".to_string(),
        profile: "base".to_string(),
        enforced: Map::new(),
        recommended: Map::new(),
        parent_profile: None,
        max_cache_age_secs: 60,
        stale_action: StaleAction::Refuse,
        version: 1,
    };
    assert_eq!(sp.max_cache_age_secs, 60);
    let _: Value = json!({ "unused": true });
}

// ── Concurrent first-write races ─────────────────────────────────────────────
//
// Mirrors `PgUserConfigStore`'s own
// `concurrent_first_writes_to_the_same_document_produce_exactly_one_winner`
// (spec 022, bug 5's regression test): two concurrent callers both supplying
// `expected_version: 0` for a not-yet-existing (app[, profile]) row race
// `FOR UPDATE` against a row that doesn't exist yet, so the race is decided
// by a unique-constraint violation on the losing `INSERT`, not by the lock.
// Each `put_*` method's own explicit `tx.rollback()` + re-read-the-real-
// winning-version handling is what exercises here — not reachable through
// any single-threaded test.

#[tokio::test]
async fn concurrent_first_put_manifest_calls_produce_exactly_one_winner() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-race-manifest");
    let store_a = PgPolicyStore::new(pool.clone());
    let store_b = PgPolicyStore::new(pool.clone());
    let verify_store = PgPolicyStore::new(pool.clone());

    let app_a = app.clone();
    let task_a = tokio::spawn(async move {
        store_a
            .put_manifest(&app_a, sample_manifest(&app_a), "alice", 0)
            .await
    });
    let app_b = app.clone();
    let task_b = tokio::spawn(async move {
        store_b
            .put_manifest(&app_b, sample_manifest(&app_b), "bob", 0)
            .await
    });

    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let results = [result_a.unwrap(), result_b.unwrap()];
    let ok_count = results.iter().filter(|r| r.is_ok()).count();
    let conflict_count = results
        .iter()
        .filter(|r| matches!(r, Err(AdminWriteError::Conflict { .. })))
        .count();
    assert_eq!(
        ok_count, 1,
        "exactly one concurrent first-write must win: {results:?}"
    );
    assert_eq!(
        conflict_count, 1,
        "the loser must see Conflict: {results:?}"
    );

    let actual = PolicyStore::manifest(&verify_store, &app)
        .await
        .unwrap()
        .unwrap();
    let winner_version = results
        .iter()
        .find_map(|r| r.as_ref().ok().copied())
        .unwrap();
    assert_eq!(actual.version, winner_version);

    let loser_current = results
        .iter()
        .find_map(|r| match r {
            Err(AdminWriteError::Conflict { current, .. }) => Some(*current),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        loser_current, actual.version,
        "the loser's Conflict.current must equal the winner's actual stored version"
    );
}

#[tokio::test]
async fn concurrent_first_put_policy_calls_produce_exactly_one_winner() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-race-policy");
    let store_a = PgPolicyStore::new(pool.clone());
    let store_b = PgPolicyStore::new(pool.clone());
    let verify_store = PgPolicyStore::new(pool.clone());

    // Fix 2 (spec 023 review): `put_policy` now locks and re-reads the
    // manifest row inside its own transaction, rejecting with
    // `MissingManifest` if none exists -- seed one up front so this race is
    // still decided by the `policy` row's own unique-constraint/lock
    // contention (what this test exists to exercise), not by both sides
    // uniformly failing validation instead.
    verify_store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();

    let app_a = app.clone();
    let task_a = tokio::spawn(async move {
        store_a
            .put_policy(
                &app_a,
                "base",
                policy_write_with_greeting("from-a"),
                MutationKind::PolicyPut,
                json!({}),
                "alice",
                0,
            )
            .await
    });
    let app_b = app.clone();
    let task_b = tokio::spawn(async move {
        store_b
            .put_policy(
                &app_b,
                "base",
                policy_write_with_greeting("from-b"),
                MutationKind::PolicyPut,
                json!({}),
                "bob",
                0,
            )
            .await
    });

    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let results = [result_a.unwrap(), result_b.unwrap()];
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "exactly one concurrent first-write must win: {results:?}"
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(AdminWriteError::Conflict { .. })))
            .count(),
        1,
        "the loser must see Conflict: {results:?}"
    );

    let actual = PolicyStore::policy(&verify_store, &app, "base")
        .await
        .unwrap()
        .unwrap();
    let winner_version = results
        .iter()
        .find_map(|r| r.as_ref().ok().copied())
        .unwrap();
    assert_eq!(actual.version, winner_version);
    // Exactly one accepted write appended exactly one log row.
    assert_eq!(count_mutation_log_rows(&pool, &app, Some("base")).await, 1);
}

#[tokio::test]
async fn concurrent_first_put_assignment_rules_calls_produce_exactly_one_winner() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-race-assignments");
    let store_a = PgPolicyStore::new(pool.clone());
    let store_b = PgPolicyStore::new(pool.clone());
    let verify_store = PgPolicyStore::new(pool.clone());

    let app_a = app.clone();
    let task_a = tokio::spawn(async move {
        store_a
            .put_assignment_rules(&app_a, vec![], "alice", 0)
            .await
    });
    let app_b = app.clone();
    let task_b =
        tokio::spawn(async move { store_b.put_assignment_rules(&app_b, vec![], "bob", 0).await });

    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let results = [result_a.unwrap(), result_b.unwrap()];
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "exactly one concurrent first-write must win: {results:?}"
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(AdminWriteError::Conflict { .. })))
            .count(),
        1,
        "the loser must see Conflict: {results:?}"
    );

    let actual_version = verify_store.assignment_rules_version(&app).await.unwrap();
    let winner_version = results
        .iter()
        .find_map(|r| r.as_ref().ok().copied())
        .unwrap();
    assert_eq!(actual_version, winner_version);
}

// ── Spec 023 review fixes: manifest/policy conformance races ───────────────
//
// Fix 1 ("High": manifest updates can invalidate already-stored policies),
// Fix 2 ("High": policy/restore validation races a concurrent manifest
// change), and Fix 3 ("Medium": partial import can strand existing DB
// policies under an incompatible imported manifest) all close the same
// underlying gap: manifest writes and policy writes previously validated
// against each other using stale, unlocked reads. See
// `src/config/service/postgres/mod.rs`'s `validate_existing_policies_against_manifest`
// (Fix 1/3's shared helper) and `PgPolicyStore::put_policy`'s own
// manifest-locking code (Fix 2) for the implementation.

/// Fix 1: `put_manifest` must reject a new manifest that would strand an
/// already-stored policy -- and must change *nothing* (the old manifest
/// stays exactly as it was) when it does.
#[tokio::test]
async fn put_manifest_rejects_a_new_manifest_that_would_invalidate_an_existing_stored_policy() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-manifest-strands-policy");
    let store = PgPolicyStore::new(pool.clone());

    // v1 declares `greeting`; a policy enforces it.
    store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();
    store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("hi"),
            MutationKind::PolicyPut,
            json!({}),
            "alice",
            0,
        )
        .await
        .unwrap();

    // v2 no longer declares `greeting` at all -- the stored `base` policy's
    // `enforced.greeting` would become an UnknownField against it.
    let v2 = ConfigManifest::new(&app, vec![]);
    let err = store.put_manifest(&app, v2, "bob", 1).await.unwrap_err();
    assert!(
        matches!(
            &err,
            AdminWriteError::Validation(errors)
                if errors.iter().any(|e| matches!(
                    e,
                    PolicyValidationError::UnknownField { path, .. } if path == "greeting"
                ))
        ),
        "expected a Validation(UnknownField) rejection, got {err:?}"
    );

    // The old manifest must still be exactly what it was -- a rejected
    // write changes nothing.
    let stored_manifest = PolicyStore::manifest(&store, &app).await.unwrap().unwrap();
    assert_eq!(
        stored_manifest.version, 1,
        "the rejected manifest write must not have landed"
    );
    assert!(
        stored_manifest.doc.leaf_by_path("greeting").is_some(),
        "the OLD manifest (still declaring 'greeting') must remain stored"
    );

    // The policy is untouched too.
    let stored_policy = PolicyStore::policy(&store, &app, "base")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored_policy.enforced.get("greeting"), Some(&json!("hi")));
    assert_eq!(stored_policy.version, 1);

    // No new mutation_log row for the rejected manifest write (only the
    // original `put_manifest` call's own row).
    assert_eq!(count_mutation_log_rows(&pool, &app, None).await, 1);
}

/// Fix 2: the genuine interleaving proof. A raw, low-level transaction
/// (bypassing `PgPolicyStore` entirely) locks the manifest row `FOR UPDATE`
/// *before* the concurrent `put_policy` call is even spawned -- deterministic
/// ordering by construction, not a timing race -- so `put_policy`'s own new
/// manifest-locking read (Fix 2's fix) is guaranteed to block on that same
/// row until this raw transaction commits its manifest update (v1, which
/// declares `greeting` -> v2, which removes it). Once it commits,
/// `put_policy`'s call resumes, re-reads the now-current v2 manifest, and
/// must reject the write it was about to make (valid against v1, invalid
/// against v2) -- proving the two operations are properly serialized rather
/// than `put_policy` racing past a stale, already-superseded manifest
/// snapshot. Given this crate's chosen lock order (manifest row locked
/// before the policy row, in both `put_manifest` and `put_policy`), admin B
/// (the manifest change) is the one guaranteed to have already committed by
/// the time admin A's (`put_policy`'s) re-validation runs, so admin A is the
/// side that must lose this race -- asserted specifically below, not just
/// "one of them failed."
#[tokio::test]
async fn put_policy_blocks_on_and_then_rejects_against_a_concurrently_committed_manifest_change() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-manifest-policy-race");
    let store = PgPolicyStore::new(pool.clone());

    // (a) Seed manifest v1 (declares `greeting`) and a policy enforcing it.
    store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();
    store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("hi"),
            MutationKind::PolicyPut,
            json!({}),
            "alice",
            0,
        )
        .await
        .unwrap();

    // (b) A raw transaction, directly against the pool (bypassing
    // `PgPolicyStore`), that locks the manifest row FOR UPDATE. This
    // `.await` fully resolves before the concurrent `put_policy` call below
    // is even spawned, so the lock is deterministically held first.
    let mut raw_tx = pool.begin().await.unwrap();
    sqlx_core::query::query::<sqlx_postgres::Postgres>(
        "SELECT doc FROM manifest WHERE app = $1 FOR UPDATE",
    )
    .bind(&app)
    .fetch_one(&mut *raw_tx)
    .await
    .unwrap();

    // (c) Concurrently, attempt a policy write that is valid against v1
    // (still enforcing `greeting`) but would be invalid against the v2
    // manifest the raw transaction is about to commit (which removes
    // `greeting` entirely). `put_policy`'s own manifest `FOR UPDATE` read
    // must block on the row lock (b) already holds.
    let app_for_task = app.clone();
    let store_for_task = PgPolicyStore::new(pool.clone());
    let task = tokio::spawn(async move {
        store_for_task
            .put_policy(
                &app_for_task,
                "base",
                policy_write_with_greeting("still-hi"),
                MutationKind::PolicyPatch,
                json!({}),
                "bob",
                1,
            )
            .await
    });

    // Not required for correctness (the row lock itself is what serializes
    // the two, regardless of timing) -- purely so the interleaving actually
    // exercised is the one this test is named for, giving the spawned task
    // a moment to reach and start blocking on its own `FOR UPDATE` before
    // the raw transaction below commits.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let v2 = ConfigManifest::new(&app, vec![]); // no longer declares `greeting`
    let v2_value = serde_json::to_value(&v2).unwrap();
    sqlx_core::query::query::<sqlx_postgres::Postgres>(
        "UPDATE manifest SET doc = $1, version = 2 WHERE app = $2",
    )
    .bind(Json(v2_value))
    .bind(&app)
    .execute(&mut *raw_tx)
    .await
    .unwrap();
    raw_tx.commit().await.unwrap();

    // (d) `put_policy` was blocked on the same manifest row lock; now that
    // the raw transaction has committed v2, it resumes, re-validates
    // against v2, and must reject -- `greeting` no longer exists in the
    // manifest it just observed.
    let result = task.await.unwrap();
    let err = result.expect_err(
        "put_policy must reject once it observes the concurrently-committed v2 manifest, \
         which no longer declares 'greeting'",
    );
    assert!(
        matches!(
            &err,
            AdminWriteError::Validation(errors)
                if errors.iter().any(|e| matches!(
                    e,
                    PolicyValidationError::UnknownField { path, .. } if path == "greeting"
                ))
        ),
        "expected a Validation(UnknownField) rejection for 'greeting', got {err:?}"
    );

    // The policy must still be exactly what it was before this race -- the
    // rejected write changed nothing.
    let stored_policy = PolicyStore::policy(&store, &app, "base")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored_policy.version, 1,
        "the rejected write must not have landed"
    );
    assert_eq!(stored_policy.enforced.get("greeting"), Some(&json!("hi")));

    // The manifest is at v2, as committed by the raw transaction -- admin
    // B's write is the one that won this race, by construction.
    let stored_manifest = PolicyStore::manifest(&store, &app).await.unwrap().unwrap();
    assert_eq!(stored_manifest.version, 2);
    assert!(stored_manifest.doc.leaf_by_path("greeting").is_none());
}

/// Fix 3: importing a bundle that declares a new manifest for an app with an
/// existing (already-in-the-target-database, NOT bundle-declared) policy
/// that the new manifest would invalidate must reject the whole import, and
/// must leave the prior manifest and policy completely untouched.
#[tokio::test]
async fn import_bundle_rejects_a_manifest_that_would_invalidate_an_existing_db_only_policy() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-import-strands-existing-policy");
    let store = PgPolicyStore::new(pool.clone());

    // Seed target-database state directly through the admin API -- a v1
    // manifest declaring `greeting`, and a policy enforcing it. The bundle
    // below will NOT mention this policy at all.
    store
        .put_manifest(&app, sample_manifest(&app), "alice", 0)
        .await
        .unwrap();
    store
        .put_policy(
            &app,
            "base",
            policy_write_with_greeting("hi"),
            MutationKind::PolicyPut,
            json!({}),
            "alice",
            0,
        )
        .await
        .unwrap();

    // A bundle that imports a NEW manifest for the same app, dropping
    // `greeting` entirely, and declares no policy for `app` at all.
    let dir = tempfile::TempDir::new().unwrap();
    write(
        &dir.path().join(format!("manifests/{app}.json")),
        &serde_json::to_string(&ConfigManifest::new(&app, vec![])).unwrap(),
    );
    let bundle = FsPolicyStore::load(dir.path()).unwrap();

    let err = store.import_bundle(&bundle, "bob").await.unwrap_err();
    assert!(
        matches!(
            &err,
            AdminWriteError::Validation(errors)
                if errors.iter().any(|e| matches!(
                    e,
                    PolicyValidationError::UnknownField { path, .. } if path == "greeting"
                ))
        ),
        "expected a Validation(UnknownField) rejection, got {err:?}"
    );

    // Both the prior manifest and the prior (bundle-unmentioned) policy must
    // be completely untouched.
    let stored_manifest = PolicyStore::manifest(&store, &app).await.unwrap().unwrap();
    assert_eq!(
        stored_manifest.version, 1,
        "the rejected import must not have landed"
    );
    assert!(stored_manifest.doc.leaf_by_path("greeting").is_some());

    let stored_policy = PolicyStore::policy(&store, &app, "base")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored_policy.enforced.get("greeting"), Some(&json!("hi")));
    assert_eq!(stored_policy.version, 1);

    assert_eq!(
        count_mutation_log_rows(&pool, &app, None).await,
        1,
        "a rejected import must append no new mutation_log row"
    );
}
