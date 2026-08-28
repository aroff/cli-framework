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
    PolicyWrite, RuleOperator, StoredManifest, StoredPolicy,
};
use cli_framework::config::StaleAction;
use serde_json::{json, Map, Value};
use sqlx_core::row::Row;
use std::path::Path;
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
