//! Postgres-backed [`PgPolicyStore`]/[`PgUserConfigStore`] — the real
//! deployment backend for the config service (spec 022).
//!
//! Built on `sqlx-core` + `sqlx-postgres` directly, never the `sqlx` facade
//! crate — see the `Cargo.toml` comment on the `sqlx-core`/`sqlx-postgres`
//! dependency declarations for the exact version/feature/TLS reasoning.
//! Every query here is hand-written against [`sqlx_core::query::query`] +
//! [`sqlx_core::row::Row::try_get`] rather than the `query!`/`query_as!`
//! compile-time-checked macros (those live in `sqlx-macros`, part of the
//! facade) or the derive-based `FromRow` (avoided to keep every column
//! access explicit and independently testable against a hand-rolled
//! `struct`).

pub mod migrations;

use super::error::{StoreError, UserConfigWriteError};
use super::store::{PolicyStore, UserConfigStore};
use super::types::{AssignmentRule, RuleOperator, StoredManifest, StoredPolicy, StoredUserConfig};
use crate::config::manifest::ConfigManifest;
use crate::config::StaleAction;
use async_trait::async_trait;
use serde_json::{Map, Value};
use sqlx_core::row::Row;
use sqlx_core::types::Json;
use sqlx_postgres::{PgRow, Postgres};

pub type PgPool = sqlx_core::pool::Pool<Postgres>;

/// Connect to `database_url` and run every pending migration (see
/// [`migrations::run_migrations`]) before returning the pool. This is the
/// one call an embedding application needs to stand up the Postgres side
/// of the config service.
pub async fn connect_and_migrate(database_url: &str) -> Result<PgPool, StoreError> {
    let pool = PgPool::connect(database_url)
        .await
        .map_err(StoreError::backend)?;
    migrations::run_migrations(&pool)
        .await
        .map_err(StoreError::backend)?;
    Ok(pool)
}

fn parse_stale_action(app: &str, raw: &str) -> Result<StaleAction, StoreError> {
    match raw {
        "warn" => Ok(StaleAction::Warn),
        "refuse" => Ok(StaleAction::Refuse),
        other => Err(StoreError::Corrupt {
            app: app.to_string(),
            message: format!("policy.stale_action has unrecognized value '{other}'"),
        }),
    }
}

fn row_to_stored_manifest(row: &PgRow) -> Result<StoredManifest, StoreError> {
    let app: String = row.try_get("app").map_err(StoreError::backend)?;
    let doc: Json<Value> = row.try_get("doc").map_err(StoreError::backend)?;
    let version: i64 = row.try_get("version").map_err(StoreError::backend)?;
    let doc: ConfigManifest = serde_json::from_value(doc.0).map_err(|e| StoreError::Corrupt {
        app: app.clone(),
        message: e.to_string(),
    })?;
    Ok(StoredManifest {
        app,
        doc,
        version: version as u64,
    })
}

fn row_to_stored_policy(row: &PgRow) -> Result<StoredPolicy, StoreError> {
    let app: String = row.try_get("app").map_err(StoreError::backend)?;
    let profile: String = row.try_get("profile").map_err(StoreError::backend)?;
    let enforced: Json<Value> = row.try_get("enforced").map_err(StoreError::backend)?;
    let recommended: Json<Value> = row.try_get("recommended").map_err(StoreError::backend)?;
    let parent_profile: Option<String> =
        row.try_get("parent_profile").map_err(StoreError::backend)?;
    let max_cache_age_secs: i64 = row
        .try_get("max_cache_age_secs")
        .map_err(StoreError::backend)?;
    let stale_action_raw: String = row.try_get("stale_action").map_err(StoreError::backend)?;
    let version: i64 = row.try_get("version").map_err(StoreError::backend)?;

    let enforced: Map<String, Value> = match enforced.0 {
        Value::Object(m) => m,
        other => {
            return Err(StoreError::Corrupt {
                app,
                message: format!("policy.enforced is not a JSON object: {other}"),
            })
        }
    };
    let recommended: Map<String, Value> = match recommended.0 {
        Value::Object(m) => m,
        other => {
            return Err(StoreError::Corrupt {
                app,
                message: format!("policy.recommended is not a JSON object: {other}"),
            })
        }
    };

    Ok(StoredPolicy {
        stale_action: parse_stale_action(&app, &stale_action_raw)?,
        app,
        profile,
        enforced,
        recommended,
        parent_profile,
        max_cache_age_secs: max_cache_age_secs as u64,
        version: version as u64,
    })
}

fn row_to_assignment_rule(row: &PgRow) -> Result<AssignmentRule, StoreError> {
    let app: String = row.try_get("app").map_err(StoreError::backend)?;
    let ord: i64 = row.try_get("ord").map_err(StoreError::backend)?;
    let claim_path: String = row.try_get("claim_path").map_err(StoreError::backend)?;
    let operator_raw: String = row.try_get("operator").map_err(StoreError::backend)?;
    let value: Option<Json<Value>> = row.try_get("value").map_err(StoreError::backend)?;
    let profile: String = row.try_get("profile").map_err(StoreError::backend)?;

    let operator =
        RuleOperator::parse_wire_str(&operator_raw).ok_or_else(|| StoreError::Corrupt {
            app: app.clone(),
            message: format!("assignment.operator has unrecognized value '{operator_raw}'"),
        })?;

    Ok(AssignmentRule {
        app,
        ord,
        claim_path,
        operator,
        value: value.map(|j| j.0),
        profile,
    })
}

/// Postgres-backed [`PolicyStore`] — see the module docs.
pub struct PgPolicyStore {
    pool: PgPool,
}

impl PgPolicyStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PolicyStore for PgPolicyStore {
    async fn manifest(&self, app: &str) -> Result<Option<StoredManifest>, StoreError> {
        let row = sqlx_core::query::query::<Postgres>(
            "SELECT app, doc, version FROM manifest WHERE app = $1",
        )
        .bind(app)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::backend)?;
        row.as_ref().map(row_to_stored_manifest).transpose()
    }

    async fn policy(&self, app: &str, profile: &str) -> Result<Option<StoredPolicy>, StoreError> {
        let row = sqlx_core::query::query::<Postgres>(
            "SELECT app, profile, enforced, recommended, parent_profile, max_cache_age_secs, \
             stale_action, version FROM policy WHERE app = $1 AND profile = $2",
        )
        .bind(app)
        .bind(profile)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::backend)?;
        row.as_ref().map(row_to_stored_policy).transpose()
    }

    async fn policies_for_app(&self, app: &str) -> Result<Vec<StoredPolicy>, StoreError> {
        let rows = sqlx_core::query::query::<Postgres>(
            "SELECT app, profile, enforced, recommended, parent_profile, max_cache_age_secs, \
             stale_action, version FROM policy WHERE app = $1",
        )
        .bind(app)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::backend)?;
        rows.iter().map(row_to_stored_policy).collect()
    }

    async fn assignment_rules(&self, app: &str) -> Result<Vec<AssignmentRule>, StoreError> {
        let rows = sqlx_core::query::query::<Postgres>(
            "SELECT app, ord, claim_path, operator, value, profile FROM assignment \
             WHERE app = $1 ORDER BY ord ASC",
        )
        .bind(app)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::backend)?;
        rows.iter().map(row_to_assignment_rule).collect()
    }

    async fn apps(&self) -> Result<Vec<String>, StoreError> {
        let rows = sqlx_core::query::query::<Postgres>(
            "SELECT app FROM manifest \
             UNION SELECT DISTINCT app FROM policy \
             UNION SELECT DISTINCT app FROM assignment \
             ORDER BY app ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::backend)?;
        rows.iter()
            .map(|r| r.try_get::<String, _>("app").map_err(StoreError::backend))
            .collect()
    }
}

/// Postgres-backed [`UserConfigStore`] — see the module docs.
pub struct PgUserConfigStore {
    pool: PgPool,
}

impl PgUserConfigStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn is_unique_violation(e: &sqlx_core::Error) -> bool {
    e.as_database_error().and_then(|de| de.code()).as_deref() == Some("23505")
}

#[async_trait]
impl UserConfigStore for PgUserConfigStore {
    async fn get(&self, app: &str, subject: &str) -> Result<StoredUserConfig, StoreError> {
        let row = sqlx_core::query::query::<Postgres>(
            "SELECT doc, version FROM user_config WHERE app = $1 AND subject = $2",
        )
        .bind(app)
        .bind(subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::backend)?;

        match row {
            None => Ok(StoredUserConfig {
                app: app.to_string(),
                subject: subject.to_string(),
                doc: Map::new(),
                version: 0,
            }),
            Some(row) => {
                let doc: Json<Value> = row.try_get("doc").map_err(StoreError::backend)?;
                let version: i64 = row.try_get("version").map_err(StoreError::backend)?;
                let doc = match doc.0 {
                    Value::Object(m) => m,
                    other => {
                        return Err(StoreError::Corrupt {
                            app: app.to_string(),
                            message: format!("user_config.doc is not a JSON object: {other}"),
                        })
                    }
                };
                Ok(StoredUserConfig {
                    app: app.to_string(),
                    subject: subject.to_string(),
                    doc,
                    version: version as u64,
                })
            }
        }
    }

    async fn put(
        &self,
        app: &str,
        subject: &str,
        doc: Map<String, Value>,
        expected_version: u64,
    ) -> Result<u64, UserConfigWriteError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| UserConfigWriteError::Store(StoreError::backend(e)))?;

        // `FOR UPDATE` serializes concurrent writers against the *existing*
        // row; the remaining race this doesn't cover on its own (two
        // concurrent *first* writes racing to INSERT the same
        // not-yet-existing (app, subject) row) is handled below by mapping
        // a unique-constraint violation on the INSERT to the same
        // `Conflict` outcome an ordinary stale `If-Match` produces — from
        // the caller's point of view, "someone else got there first" reads
        // identically either way.
        let existing = sqlx_core::query::query::<Postgres>(
            "SELECT version FROM user_config WHERE app = $1 AND subject = $2 FOR UPDATE",
        )
        .bind(app)
        .bind(subject)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| UserConfigWriteError::Store(StoreError::backend(e)))?;

        let current_version: u64 = match &existing {
            Some(row) => row
                .try_get::<i64, _>("version")
                .map_err(|e| UserConfigWriteError::Store(StoreError::backend(e)))?
                as u64,
            None => 0,
        };

        if current_version != expected_version {
            return Err(UserConfigWriteError::Conflict {
                current: current_version,
                expected: expected_version,
            });
        }

        let new_version = current_version + 1;
        let doc_json = Json(Value::Object(doc));

        let write_result = if existing.is_some() {
            sqlx_core::query::query::<Postgres>(
                "UPDATE user_config SET doc = $1, version = $2 WHERE app = $3 AND subject = $4",
            )
            .bind(doc_json)
            .bind(new_version as i64)
            .bind(app)
            .bind(subject)
            .execute(&mut *tx)
            .await
        } else {
            sqlx_core::query::query::<Postgres>(
                "INSERT INTO user_config (app, subject, doc, version) VALUES ($1, $2, $3, $4)",
            )
            .bind(app)
            .bind(subject)
            .bind(doc_json)
            .bind(new_version as i64)
            .execute(&mut *tx)
            .await
        };

        if let Err(e) = write_result {
            if !is_unique_violation(&e) {
                return Err(UserConfigWriteError::Store(StoreError::backend(e)));
            }

            // A unique-constraint violation means another writer's INSERT
            // for this exact (app, subject) won the race between our own
            // `FOR UPDATE` read (which saw no row) and our INSERT above.
            // This transaction is now aborted -- Postgres refuses any
            // further command on it until rolled back -- so roll it back
            // explicitly, then re-read the row's actual, just-committed
            // version on a fresh connection/statement. Bug 5's fix: the
            // trait contract promises `current` is the document's real
            // stored version at the time of conflict, and the previous
            // hardcoded `current: 0` was never actually read back from the
            // database, so it was wrong for any first-write race whose
            // winner didn't happen to land at version 1 by coincidence (and
            // undocumented-misleading even when it did).
            let _ = tx.rollback().await;
            let winner_row = sqlx_core::query::query::<Postgres>(
                "SELECT version FROM user_config WHERE app = $1 AND subject = $2",
            )
            .bind(app)
            .bind(subject)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| UserConfigWriteError::Store(StoreError::backend(e)))?;
            let real_current = match winner_row {
                Some(row) => row
                    .try_get::<i64, _>("version")
                    .map_err(|e| UserConfigWriteError::Store(StoreError::backend(e)))?
                    as u64,
                // Extremely unlikely (a unique violation implies a row now
                // exists), but fall back to this writer's own prior view
                // rather than panicking if the row is somehow gone by the
                // time it's re-read.
                None => current_version,
            };
            return Err(UserConfigWriteError::Conflict {
                current: real_current,
                expected: expected_version,
            });
        }

        tx.commit()
            .await
            .map_err(|e| UserConfigWriteError::Store(StoreError::backend(e)))?;

        Ok(new_version)
    }
}

/// Test/dev-only seeding and reset helpers — **not** an administrative
/// write API (spec 022 explicitly excludes one; spec 023 owns that). These
/// exist so the Postgres conformance suite
/// (`tests/integration/config_service_postgres_conformance.rs`) can put the
/// database into the same states `FsPolicyStore` reads straight out of a
/// bundle directory, exercising the identical trait-level scenario set
/// against both backends rather than two hand-written, potentially
/// drifting copies. Gated behind `testkit` (the same feature every other
/// test-only surface in this crate uses) so it never ships in a normal
/// build, and only usable from an external test binary, which cannot see
/// `#[cfg(test)]` items in this library crate.
#[cfg(feature = "testkit")]
pub mod testkit {
    use super::*;

    /// The inverse of [`parse_stale_action`] — only needed for seeding
    /// (production reads always go the other direction), which is why it
    /// lives here rather than beside its inverse above.
    fn stale_action_wire_str(action: StaleAction) -> &'static str {
        match action {
            StaleAction::Warn => "warn",
            StaleAction::Refuse => "refuse",
        }
    }

    /// Remove every row from every config-service table — for isolating
    /// tests that share one long-lived Postgres instance (CI's service
    /// container, or a developer's local database) rather than spinning up
    /// a fresh database per test.
    pub async fn truncate_all(pool: &PgPool) -> Result<(), StoreError> {
        sqlx_core::raw_sql::raw_sql("TRUNCATE TABLE manifest, policy, assignment, user_config")
            .execute(pool)
            .await
            .map_err(StoreError::backend)?;
        Ok(())
    }

    pub async fn seed_manifest(pool: &PgPool, manifest: &StoredManifest) -> Result<(), StoreError> {
        let doc = serde_json::to_value(&manifest.doc).map_err(StoreError::backend)?;
        sqlx_core::query::query::<Postgres>(
            "INSERT INTO manifest (app, doc, version) VALUES ($1, $2, $3) \
             ON CONFLICT (app) DO UPDATE SET doc = EXCLUDED.doc, version = EXCLUDED.version",
        )
        .bind(&manifest.app)
        .bind(Json(doc))
        .bind(manifest.version as i64)
        .execute(pool)
        .await
        .map_err(StoreError::backend)?;
        Ok(())
    }

    pub async fn seed_policy(pool: &PgPool, policy: &StoredPolicy) -> Result<(), StoreError> {
        sqlx_core::query::query::<Postgres>(
            "INSERT INTO policy (app, profile, enforced, recommended, parent_profile, \
             max_cache_age_secs, stale_action, version) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (app, profile) DO UPDATE SET \
               enforced = EXCLUDED.enforced, recommended = EXCLUDED.recommended, \
               parent_profile = EXCLUDED.parent_profile, \
               max_cache_age_secs = EXCLUDED.max_cache_age_secs, \
               stale_action = EXCLUDED.stale_action, version = EXCLUDED.version",
        )
        .bind(&policy.app)
        .bind(&policy.profile)
        .bind(Json(Value::Object(policy.enforced.clone())))
        .bind(Json(Value::Object(policy.recommended.clone())))
        .bind(&policy.parent_profile)
        .bind(policy.max_cache_age_secs as i64)
        .bind(stale_action_wire_str(policy.stale_action))
        .bind(policy.version as i64)
        .execute(pool)
        .await
        .map_err(StoreError::backend)?;
        Ok(())
    }

    pub async fn seed_assignment_rule(
        pool: &PgPool,
        rule: &AssignmentRule,
    ) -> Result<(), StoreError> {
        sqlx_core::query::query::<Postgres>(
            "INSERT INTO assignment (app, ord, claim_path, operator, value, profile) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (app, ord) DO UPDATE SET \
               claim_path = EXCLUDED.claim_path, operator = EXCLUDED.operator, \
               value = EXCLUDED.value, profile = EXCLUDED.profile",
        )
        .bind(&rule.app)
        .bind(rule.ord)
        .bind(&rule.claim_path)
        .bind(rule.operator.wire_str())
        .bind(rule.value.clone().map(Json))
        .bind(&rule.profile)
        .execute(pool)
        .await
        .map_err(StoreError::backend)?;
        Ok(())
    }
}

/// Live-Postgres-only tests exercising the corners a healthy, well-formed
/// database never reaches through the ordinary `PolicyStore`/
/// `UserConfigStore` trait methods: corrupt row contents (someone edited
/// the database directly, or a future version wrote a shape this binary
/// doesn't understand), the concurrent-first-write race
/// `is_unique_violation` exists for, and `testkit::truncate_all` itself.
/// Same `DATABASE_URL`-presence skip contract as everywhere else in this
/// feature; see `tests/integration/config_service_postgres_conformance.rs`'s
/// module docs for the full rationale.
#[cfg(all(test, feature = "testkit"))]
mod tests {
    use super::testkit;
    use super::*;
    use uuid::Uuid;

    fn database_url_or_skip() -> Option<String> {
        match std::env::var("DATABASE_URL") {
            Ok(u) => Some(u),
            Err(_) => {
                eprintln!("skipping postgres::tests: DATABASE_URL is not set");
                None
            }
        }
    }

    fn unique(prefix: &str) -> String {
        format!("{prefix}_{}", Uuid::new_v4().simple())
    }

    #[test]
    fn parse_stale_action_rejects_an_unrecognized_value() {
        let err = parse_stale_action("myapp", "not-a-real-action").unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { .. }));
        assert!(parse_stale_action("myapp", "warn").is_ok());
        assert!(parse_stale_action("myapp", "refuse").is_ok());
    }

    #[tokio::test]
    async fn manifest_with_corrupt_doc_json_is_a_corrupt_error() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let pool = connect_and_migrate(&url).await.unwrap();
        let app = unique("corrupt_manifest_app");

        // Valid JSON, but not a `ConfigManifest` (missing required fields)
        // -- the row exists and decodes as JSON fine, only the second-stage
        // `serde_json::from_value::<ConfigManifest>` fails.
        sqlx_core::query::query::<Postgres>(
            "INSERT INTO manifest (app, doc, version) VALUES ($1, $2, 1)",
        )
        .bind(&app)
        .bind(Json(serde_json::json!({"not": "a manifest"})))
        .execute(&pool)
        .await
        .unwrap();

        let store = PgPolicyStore::new(pool);
        let err = store.manifest(&app).await.unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn policy_with_non_object_enforced_or_recommended_is_a_corrupt_error() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let pool = connect_and_migrate(&url).await.unwrap();
        let app = unique("corrupt_policy_app");

        sqlx_core::query::query::<Postgres>(
            "INSERT INTO policy (app, profile, enforced, recommended, version) \
             VALUES ($1, 'base', $2, '{}'::jsonb, 1)",
        )
        .bind(&app)
        .bind(Json(serde_json::json!(["not", "an", "object"])))
        .execute(&pool)
        .await
        .unwrap();

        let store = PgPolicyStore::new(pool.clone());
        let err = store.policy(&app, "base").await.unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { .. }), "got {err:?}");

        let app2 = unique("corrupt_policy_app2");
        sqlx_core::query::query::<Postgres>(
            "INSERT INTO policy (app, profile, enforced, recommended, version) \
             VALUES ($1, 'base', '{}'::jsonb, $2, 1)",
        )
        .bind(&app2)
        .bind(Json(serde_json::json!("not an object either")))
        .execute(&pool)
        .await
        .unwrap();
        let err2 = store.policy(&app2, "base").await.unwrap_err();
        assert!(matches!(err2, StoreError::Corrupt { .. }), "got {err2:?}");
    }

    #[tokio::test]
    async fn assignment_rule_with_an_unrecognized_operator_is_a_corrupt_error() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let pool = connect_and_migrate(&url).await.unwrap();
        let app = unique("corrupt_assignment_app");

        sqlx_core::query::query::<Postgres>(
            "INSERT INTO assignment (app, ord, claim_path, operator, profile) \
             VALUES ($1, 0, 'team', 'startswith', 'p')",
        )
        .bind(&app)
        .execute(&pool)
        .await
        .unwrap();

        let store = PgPolicyStore::new(pool);
        let err = store.assignment_rules(&app).await.unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn concurrent_first_writes_to_the_same_document_produce_exactly_one_winner() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let pool = connect_and_migrate(&url).await.unwrap();
        let app = unique("race_app");
        let subject = "u1".to_string();

        let store_a = PgUserConfigStore::new(pool.clone());
        let store_b = PgUserConfigStore::new(pool.clone());
        // A third, independent handle used only *after* the race settles,
        // to read the row back and confirm what the loser was actually
        // told matches reality (bug 5) rather than trusting the race
        // itself to prove it.
        let verify_store = PgUserConfigStore::new(pool);

        let app_a = app.clone();
        let subject_a = subject.clone();
        let task_a = tokio::spawn(async move {
            store_a
                .put(&app_a, &subject_a, serde_json::Map::new(), 0)
                .await
        });
        let app_b = app.clone();
        let subject_b = subject.clone();
        let task_b = tokio::spawn(async move {
            store_b
                .put(&app_b, &subject_b, serde_json::Map::new(), 0)
                .await
        });

        let (result_a, result_b) = tokio::join!(task_a, task_b);
        let results = [result_a.unwrap(), result_b.unwrap()];
        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        let conflict_count = results
            .iter()
            .filter(|r| matches!(r, Err(UserConfigWriteError::Conflict { .. })))
            .count();
        assert_eq!(
            ok_count, 1,
            "exactly one concurrent first-write must win: {results:?}"
        );
        assert_eq!(
            conflict_count, 1,
            "the loser must see Conflict, not some other error: {results:?}"
        );

        // Bug 5: the loser's `Conflict.current` must equal the winner's
        // *actual* stored version -- independently read back from the
        // database, not the hardcoded `0` the pre-fix code always
        // reported regardless of what really won.
        let actual = verify_store.get(&app, &subject).await.unwrap();
        let winner_version = results
            .iter()
            .find_map(|r| r.as_ref().ok().copied())
            .expect("exactly one winner");
        assert_eq!(
            actual.version, winner_version,
            "sanity check: the independently-read-back version must equal what the winning put() returned"
        );

        let loser_conflict_current = results
            .iter()
            .find_map(|r| match r {
                Err(UserConfigWriteError::Conflict { current, .. }) => Some(*current),
                _ => None,
            })
            .expect("exactly one loser");
        assert_eq!(
            loser_conflict_current, actual.version,
            "the loser's Conflict.current must equal the winner's actual stored version, not a hardcoded value"
        );
    }

    #[tokio::test]
    async fn truncate_all_removes_every_row_from_every_table() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        // `truncate_all` is database-wide, not scoped to one app -- unlike
        // every other test in this module, it cannot share the ordinary
        // "postgres" database with tests running concurrently in other
        // binaries/processes without clobbering their fixtures. It gets its
        // own throwaway database for exactly that reason.
        let idx = url.rfind('/').unwrap();
        let admin_pool = PgPool::connect(&format!("{}/postgres", &url[..idx]))
            .await
            .unwrap();
        let db_name = format!("cfw022_pgtest_{}", Uuid::new_v4().simple());
        sqlx_core::raw_sql::raw_sql(&format!("CREATE DATABASE {db_name}"))
            .execute(&admin_pool)
            .await
            .unwrap();
        let fresh_url = format!("{}/{db_name}", &url[..idx]);
        let pool = connect_and_migrate(&fresh_url).await.unwrap();

        let manifest = StoredManifest {
            app: "app".to_string(),
            doc: ConfigManifest::new("app", vec![]),
            version: 1,
        };
        testkit::seed_manifest(&pool, &manifest).await.unwrap();
        testkit::seed_policy(
            &pool,
            &StoredPolicy {
                app: "app".to_string(),
                profile: "base".to_string(),
                enforced: Map::new(),
                recommended: Map::new(),
                parent_profile: None,
                max_cache_age_secs: 3600,
                stale_action: StaleAction::Warn,
                version: 1,
            },
        )
        .await
        .unwrap();

        let store = PgPolicyStore::new(pool.clone());
        assert!(store.manifest("app").await.unwrap().is_some());

        testkit::truncate_all(&pool).await.unwrap();

        assert!(store.manifest("app").await.unwrap().is_none());
        assert!(store.apps().await.unwrap().is_empty());

        pool.close().await;
        let _ =
            sqlx_core::raw_sql::raw_sql(&format!("DROP DATABASE IF EXISTS {db_name} WITH (FORCE)"))
                .execute(&admin_pool)
                .await;
    }
}
