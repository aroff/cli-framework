//! Trait-conformance suite for `PolicyStore` and `UserConfigStore`: the same
//! scenario set is asserted against both the bundle-directory
//! (`FsPolicyStore`) and in-memory (`InMemoryUserConfigStore`) test
//! implementations *and* the real Postgres implementations
//! (`PgPolicyStore`/`PgUserConfigStore`), so the trait contract is pinned
//! once rather than twice, inconsistently (spec 022, "Postgres testing").
//!
//! Unlike the pre-existing `secrets_openbao_conformance.rs` precedent (which
//! is `testcontainers`-based, self-skips locally, and **never runs in CI at
//! all**), this suite is triggered purely by the presence of `DATABASE_URL`
//! in the environment — no separate opt-in flag — because CI always sets it
//! (via the `postgres:` service container added to `.github/workflows/ci.yml`)
//! while local dev usually has no stray Postgres running. **This suite is
//! meant to actually execute in CI**, which is the entire point of it being
//! different from the OpenBao precedent.
//!
//! Run it locally against a real Postgres:
//! ```sh
//! docker run --rm -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:16
//! DATABASE_URL=postgresql://postgres:postgres@localhost:5432/postgres \
//!     cargo test --features config-service,testkit --test integration_config_service_postgres_conformance
//! ```
//!
//! Test isolation: every test claims a globally-unique app name (and, for
//! roaming documents, subject name) via `uuid::Uuid::new_v4()` rather than
//! truncating shared tables between tests — `#[tokio::test]` functions in
//! one binary run concurrently by default, and a shared `TRUNCATE` between
//! parallel tests hitting the *same* long-lived database (CI's service
//! container, or a developer's local instance) would make them clobber each
//! other's fixtures. Unique names sidestep that without serializing the
//! suite. `cli_framework::config::service::postgres::testkit::truncate_all`
//! still exists as a manual reset a developer can reach for locally; this
//! suite does not call it itself.

use cli_framework::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
use cli_framework::config::service::postgres::{
    connect_and_migrate, testkit, PgPolicyStore, PgPool, PgUserConfigStore,
};
use cli_framework::config::service::{
    AssignmentRule, PolicyStore, RuleOperator, StoredManifest, StoredPolicy, UserConfigStore,
    UserConfigWriteError,
};
use cli_framework::config::StaleAction;
use serde_json::{json, Map};
use std::path::Path;
use uuid::Uuid;

async fn pool_or_skip() -> Option<PgPool> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping Postgres conformance suite: DATABASE_URL is not set. \
             CI always sets this (see the `postgres` service container in \
             .github/workflows/ci.yml); local dev usually doesn't have a \
             stray Postgres running, which is expected -- this is a graceful \
             skip, not a failure."
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

// ── Shared fixture ───────────────────────────────────────────────────────────

fn fixture_manifest(app: &str) -> ConfigManifest {
    ConfigManifest::new(
        app,
        vec![FieldManifest {
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
        }],
    )
}

fn fixture_base_policy(app: &str) -> StoredPolicy {
    let mut enforced = Map::new();
    enforced.insert("greeting".to_string(), json!("hello from base"));
    StoredPolicy {
        app: app.to_string(),
        profile: "base".to_string(),
        enforced,
        recommended: Map::new(),
        parent_profile: None,
        max_cache_age_secs: 3600,
        stale_action: StaleAction::Warn,
        version: 1,
    }
}

fn fixture_child_policy(app: &str) -> StoredPolicy {
    StoredPolicy {
        app: app.to_string(),
        profile: "child".to_string(),
        enforced: Map::new(),
        recommended: Map::new(),
        parent_profile: Some("base".to_string()),
        max_cache_age_secs: 60,
        stale_action: StaleAction::Refuse,
        version: 3,
    }
}

fn fixture_rule(app: &str) -> AssignmentRule {
    AssignmentRule {
        app: app.to_string(),
        ord: 0,
        claim_path: "realm_access.roles".to_string(),
        operator: RuleOperator::Contains,
        value: Some(json!("developers")),
        profile: "child".to_string(),
    }
}

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// The `FsPolicyStore` side of the fixture: the same three "documents"
/// as the Postgres seed below, expressed as bundle files.
fn write_fs_fixture(root: &Path, app: &str) {
    write(
        &root.join(format!("manifests/{app}.json")),
        &serde_json::to_string(&fixture_manifest(app)).unwrap(),
    );
    write(
        &root.join(format!("policies/{app}/base.toml")),
        r#"
        version = 1
        [enforced]
        "greeting" = "hello from base"
        "#,
    );
    write(
        &root.join(format!("policies/{app}/child.toml")),
        r#"
        version = 3
        parent_profile = "base"
        max_cache_age_secs = 60
        stale_action = "refuse"
        "#,
    );
    write(
        &root.join("assignments.toml"),
        &format!(
            r#"
            [{app}]
            [[{app}.rules]]
            claim_path = "realm_access.roles"
            operator = "contains"
            value = "developers"
            profile = "child"
            "#
        ),
    );
}

// ── PolicyStore conformance ──────────────────────────────────────────────────

/// The scenario set both `PolicyStore` implementations must satisfy
/// identically, given `app` has already been seeded with
/// [`fixture_manifest`]/[`fixture_base_policy`]/[`fixture_child_policy`]/[`fixture_rule`].
async fn assert_policy_store_conformance(store: &dyn PolicyStore, app: &str) {
    let manifest = store
        .manifest(app)
        .await
        .unwrap()
        .expect("manifest must be present");
    assert_eq!(manifest.app, app);
    assert_eq!(manifest.doc.iter_leaves().len(), 1);

    let base = store
        .policy(app, "base")
        .await
        .unwrap()
        .expect("base policy must be present");
    assert_eq!(
        base.enforced.get("greeting"),
        Some(&json!("hello from base"))
    );
    assert!(base.parent_profile.is_none());

    let child = store
        .policy(app, "child")
        .await
        .unwrap()
        .expect("child policy must be present");
    assert_eq!(child.parent_profile.as_deref(), Some("base"));
    assert_eq!(child.max_cache_age_secs, 60);
    assert_eq!(child.stale_action, StaleAction::Refuse);

    assert!(store
        .policy(app, "no-such-profile")
        .await
        .unwrap()
        .is_none());

    let mut all = store.policies_for_app(app).await.unwrap();
    all.sort_by(|a, b| a.profile.cmp(&b.profile));
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].profile, "base");
    assert_eq!(all[1].profile, "child");

    let rules = store.assignment_rules(app).await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].operator, RuleOperator::Contains);
    assert_eq!(rules[0].profile, "child");

    let apps = store.apps().await.unwrap();
    assert!(apps.contains(&app.to_string()));
}

#[tokio::test]
async fn fs_policy_store_satisfies_the_conformance_suite() {
    let app = unique("fs-app");
    let dir = tempfile::TempDir::new().unwrap();
    write_fs_fixture(dir.path(), &app);
    let store = cli_framework::config::service::FsPolicyStore::load(dir.path()).unwrap();
    assert_policy_store_conformance(&store, &app).await;
}

#[tokio::test]
async fn pg_policy_store_satisfies_the_conformance_suite() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("pg-app");

    testkit::seed_manifest(
        &pool,
        &StoredManifest {
            app: app.clone(),
            doc: fixture_manifest(&app),
            version: 1,
        },
    )
    .await
    .unwrap();
    testkit::seed_policy(&pool, &fixture_base_policy(&app))
        .await
        .unwrap();
    testkit::seed_policy(&pool, &fixture_child_policy(&app))
        .await
        .unwrap();
    testkit::seed_assignment_rule(&pool, &fixture_rule(&app))
        .await
        .unwrap();

    let store = PgPolicyStore::new(pool);
    assert_policy_store_conformance(&store, &app).await;
}

// ── UserConfigStore conformance ──────────────────────────────────────────────

/// The scenario set both `UserConfigStore` implementations must satisfy
/// identically. `app`/`subject` must not have been written to before this
/// runs (a fresh unique pair per test achieves that without needing to
/// reset shared state).
async fn assert_user_config_store_conformance(
    store: &dyn UserConfigStore,
    app: &str,
    subject: &str,
) {
    let initial = store.get(app, subject).await.unwrap();
    assert_eq!(initial.version, 0);
    assert!(initial.doc.is_empty());

    let mut doc = Map::new();
    doc.insert("greeting".to_string(), json!("hello"));
    let v1 = store.put(app, subject, doc.clone(), 0).await.unwrap();
    assert_eq!(v1, 1);

    let after_first_write = store.get(app, subject).await.unwrap();
    assert_eq!(after_first_write.doc, doc);
    assert_eq!(after_first_write.version, 1);

    // Stale expected_version is rejected and leaves the stored document
    // unchanged -- the core optimistic-concurrency guarantee spec 021's
    // roaming client depends on.
    let mut conflicting = Map::new();
    conflicting.insert("greeting".to_string(), json!("should not land"));
    let err = store.put(app, subject, conflicting, 0).await.unwrap_err();
    assert!(matches!(
        err,
        UserConfigWriteError::Conflict {
            current: 1,
            expected: 0
        }
    ));

    let unchanged = store.get(app, subject).await.unwrap();
    assert_eq!(unchanged.doc, doc, "conflicting write must not have landed");
    assert_eq!(unchanged.version, 1);

    // The correct expected_version succeeds and advances the version.
    let mut second = Map::new();
    second.insert("greeting".to_string(), json!("updated"));
    let v2 = store.put(app, subject, second.clone(), 1).await.unwrap();
    assert_eq!(v2, 2);
    let after_second_write = store.get(app, subject).await.unwrap();
    assert_eq!(after_second_write.doc, second);

    // A different subject is isolated.
    let other = store
        .get(app, "a-different-subject-entirely")
        .await
        .unwrap();
    assert!(other.doc.is_empty());
}

#[tokio::test]
async fn in_memory_user_config_store_satisfies_the_conformance_suite() {
    let store = cli_framework::config::service::InMemoryUserConfigStore::new();
    assert_user_config_store_conformance(&store, &unique("app"), &unique("subject")).await;
}

#[tokio::test]
async fn pg_user_config_store_satisfies_the_conformance_suite() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let store = PgUserConfigStore::new(pool);
    assert_user_config_store_conformance(&store, &unique("app"), &unique("subject")).await;
}

// ── Migration runner ─────────────────────────────────────────────────────────

#[tokio::test]
async fn migration_runner_is_idempotent_running_it_twice_is_a_no_op() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    // First connection already ran migrations (possibly by an earlier test
    // in this binary, possibly fresh) -- running it again from a second,
    // independent pool must not error.
    let _pool = connect_and_migrate(&url)
        .await
        .expect("first connect + migrate");
    let _pool2 = connect_and_migrate(&url)
        .await
        .expect("second connect + migrate must be a no-op");
}
