//! A small, honest, hand-rolled SQL migration runner (spec 022, "Schema and
//! migrations") — there is no existing SQL migration convention anywhere in
//! this workspace (no `migrations/` directory, no `sqlx::migrate!` usage
//! anywhere) to follow, and `sqlx::migrate!` lives in the `sqlx` facade
//! crate this feature deliberately does not depend on.
//!
//! Mirrors the *philosophy* `ConfigStore`'s own JSON-document migrations
//! already use (`src/config/store.rs`: "a document claiming a version ahead
//! of `current_version` must be refused, never downgraded or migrated as if
//! it were old") — not that code, since this is SQL, not JSON, and the
//! shapes involved (an ordered list of embedded `.sql` scripts vs. a chain
//! of `Fn(Value) -> Value` closures) don't share an implementation.
//!
//! Each migration is embedded as `&'static str` via `include_str!` and
//! tracked in a `schema_migrations(version, applied_at)` table. Running
//! forward applies every migration strictly greater than the database's
//! current version, in order, each inside its own transaction. A database
//! whose recorded version is *ahead* of what this binary knows about
//! refuses to serve rather than silently skipping ahead — the same
//! "refuse, don't downgrade" rule `ConfigStore::load` already enforces for
//! its own version field.

use super::PgPool;
use crate::config::service::error::StoreError;
use sqlx_core::connection::Connection;
use sqlx_core::row::Row;
use sqlx_postgres::PgConnection;

/// Every migration this binary knows about, in order. `1` is the initial
/// schema (spec 022's four tables plus `schema_migrations` itself).
pub const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("migrations/001_initial.sql"))];

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MigrationError {
    #[error("database schema is at version {found}, but this binary only knows migrations up to version {known}. Refusing to serve rather than guess how to handle a newer schema.")]
    DatabaseAheadOfBinary { found: i64, known: i64 },
    #[error("no migration takes the schema from version {from} to {to} (gap in the embedded migration list)")]
    Gap { from: i64, to: i64 },
    #[error("migration {version} failed: {message}")]
    Failed { version: i64, message: String },
}

/// An arbitrary, fixed key for a Postgres advisory lock scoped to this
/// migration runner — the specific number carries no meaning beyond being
/// unique enough not to collide with some *other* application's advisory
/// lock on the same database (spec 022 user story 28, "run multiple
/// replicas": every replica calls this at startup against the same
/// database, and without serializing them, two replicas racing to migrate a
/// brand-new database both attempt `CREATE TABLE` and one gets a duplicate-
/// object error — reproduced against a real Postgres instance while
/// building this feature, not a hypothetical).
const MIGRATION_LOCK_KEY: i64 = 0x636C695F303232; // "cli_022" as bytes, as a plain i64

/// Run every migration in [`MIGRATIONS`] not yet applied to `pool`'s
/// database, in order, each inside its own transaction. Bootstraps
/// `schema_migrations` itself as part of migration `1` — before that table
/// exists, "current version" is determined by whether it exists at all
/// (via `to_regclass`, which returns `NULL` rather than erroring when the
/// table is absent, so this bootstrap check needs no separate
/// "does this table exist" special case beyond the query itself).
///
/// Checks out exactly **one** connection from `pool` for the entire run
/// ([`MIGRATION_LOCK_KEY`]'s session-level advisory lock, `current_version`,
/// and every migration's `BEGIN`/apply/`COMMIT`) and runs everything on
/// that single connection. Bug fix, replacing an earlier version of this
/// doc comment that claimed the lock connection's scope was just the lock
/// itself: that version checked out the lock on one connection but then
/// called `run_migrations_holding_lock(pool)`, which acquired *additional*
/// connections from the same pool for `current_version` and `pool.begin()`.
/// Under a minimally-sized pool (`max_connections(1)`, or enough concurrent
/// replicas that the pool is otherwise saturated), the connection the
/// migration queries actually needed could never be checked out — the
/// lock-holding connection was never released until the migration finished,
/// so the run deadlocked against itself (verified against a real Postgres
/// instance with a `max_connections(1)` pool, not a hypothetical). Doing
/// everything on the one checked-out connection removes the second
/// acquisition entirely, so a pool of size 1 is sufficient. Two replicas
/// racing to migrate the same fresh database still serialize correctly on
/// the advisory lock, each still holding only its own single connection.
/// The lock is released whether the migration run succeeds or fails.
pub async fn run_migrations(pool: &PgPool) -> Result<(), MigrationError> {
    let mut lock_conn = pool.acquire().await.map_err(|e| MigrationError::Failed {
        version: 0,
        message: format!("acquiring migration-lock connection: {e}"),
    })?;

    sqlx_core::query::query::<sqlx_postgres::Postgres>("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await
        .map_err(|e| MigrationError::Failed {
            version: 0,
            message: format!("acquiring migration advisory lock: {e}"),
        })?;

    let result = run_migrations_holding_lock(&mut lock_conn).await;

    // Always attempt to release, even on failure -- otherwise a failed
    // migration would hold the lock for as long as this pooled connection
    // stays checked out, wedging every future caller against the same
    // database.
    let _ = sqlx_core::query::query::<sqlx_postgres::Postgres>("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await;

    result
}

async fn run_migrations_holding_lock(conn: &mut PgConnection) -> Result<(), MigrationError> {
    run_migrations_with(conn, MIGRATIONS).await
}

/// The actual migration-application logic, parameterized over the
/// migration list so tests can exercise [`MigrationError::Gap`] with a
/// synthetic list that skips a version — [`MIGRATIONS`] itself only ever
/// has one entry today, so that branch is otherwise unreachable until a
/// second real migration ships. [`run_migrations_holding_lock`] is the only
/// non-test caller, and always passes [`MIGRATIONS`].
///
/// Takes `conn` as an already-checked-out connection, never a `&PgPool` —
/// see [`run_migrations`]'s doc comment for why acquiring a *second*
/// connection from the pool here (the pre-fix behavior) is exactly the bug
/// this signature change closes.
async fn run_migrations_with(
    conn: &mut PgConnection,
    migrations: &[(i64, &str)],
) -> Result<(), MigrationError> {
    let mut current = current_version(&mut *conn)
        .await
        .map_err(|e| MigrationError::Failed {
            version: 0,
            message: e.to_string(),
        })?;

    let known_max = migrations.iter().map(|(v, _)| *v).max().unwrap_or(0);
    if current > known_max {
        return Err(MigrationError::DatabaseAheadOfBinary {
            found: current,
            known: known_max,
        });
    }

    let mut sorted: Vec<&(i64, &str)> = migrations.iter().collect();
    sorted.sort_by_key(|(v, _)| *v);

    for (version, sql) in sorted {
        if *version <= current {
            continue;
        }
        if *version != current + 1 {
            return Err(MigrationError::Gap {
                from: current,
                to: *version,
            });
        }

        // A transaction on `conn` itself (`Connection::begin`), not a
        // second connection acquired from a pool -- see the module-level
        // rationale on `run_migrations`.
        let mut tx = conn.begin().await.map_err(|e| MigrationError::Failed {
            version: *version,
            message: e.to_string(),
        })?;

        sqlx_core::raw_sql::raw_sql(sql)
            .execute(&mut *tx)
            .await
            .map_err(|e| MigrationError::Failed {
                version: *version,
                message: e.to_string(),
            })?;

        // `schema_migrations` only exists once migration 1 has itself run
        // (it's created inside 001_initial.sql), so this insert is safe for
        // every migration including the first.
        sqlx_core::query::query::<sqlx_postgres::Postgres>(
            "INSERT INTO schema_migrations (version) VALUES ($1)",
        )
        .bind(*version)
        .execute(&mut *tx)
        .await
        .map_err(|e| MigrationError::Failed {
            version: *version,
            message: e.to_string(),
        })?;

        tx.commit().await.map_err(|e| MigrationError::Failed {
            version: *version,
            message: e.to_string(),
        })?;

        current = *version;
    }

    Ok(())
}

/// The database's current schema version: `0` if `schema_migrations`
/// doesn't exist yet (a brand-new database), otherwise `MAX(version)`.
/// Takes an already-checked-out connection rather than a `&PgPool` — see
/// [`run_migrations`]'s doc comment.
async fn current_version(conn: &mut PgConnection) -> Result<i64, StoreError> {
    let row = sqlx_core::query::query::<sqlx_postgres::Postgres>(
        "SELECT to_regclass('public.schema_migrations') IS NOT NULL AS table_exists",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(StoreError::backend)?;
    let exists: bool = row.try_get("table_exists").map_err(StoreError::backend)?;
    if !exists {
        return Ok(0);
    }

    let row = sqlx_core::query::query::<sqlx_postgres::Postgres>(
        "SELECT COALESCE(MAX(version), 0) AS max_version FROM schema_migrations",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(StoreError::backend)?;
    row.try_get::<i64, _>("max_version")
        .map_err(StoreError::backend)
}

/// Live-Postgres-only tests: skip gracefully when `DATABASE_URL` is unset,
/// same trigger and rationale as
/// `tests/integration/config_service_postgres_conformance.rs` (see that
/// file's module docs). Unlike that external suite, these need a genuinely
/// **fresh, empty database** (no `schema_migrations` table at all) to
/// exercise the bootstrap and gap/ahead-of-binary error paths, which the
/// shared `postgres` database the other conformance tests share cannot
/// give without racing them — so each test here creates and drops its own
/// throwaway database via `CREATE DATABASE`/`DROP DATABASE`.
#[cfg(test)]
mod tests {
    use super::*;

    fn admin_url(database_url: &str) -> String {
        let idx = database_url
            .rfind('/')
            .expect("DATABASE_URL must contain a path segment");
        format!("{}/postgres", &database_url[..idx])
    }

    fn with_db_name(database_url: &str, db_name: &str) -> String {
        let idx = database_url
            .rfind('/')
            .expect("DATABASE_URL must contain a path segment");
        format!("{}/{db_name}", &database_url[..idx])
    }

    /// Creates a uniquely-named, genuinely empty database, hands the caller
    /// a pool connected to it, and drops the database again on the way out
    /// (best-effort -- a cleanup failure is logged, not panicked on, so it
    /// never masks the test's own assertion failures).
    struct FreshDatabase {
        admin_pool: PgPool,
        db_name: String,
        pub pool: PgPool,
    }

    impl FreshDatabase {
        async fn create(database_url: &str) -> Self {
            Self::create_with_pool_options(
                database_url,
                sqlx_core::pool::PoolOptions::<sqlx_postgres::Postgres>::new(),
            )
            .await
        }

        /// Like [`Self::create`], but connects the returned pool with a
        /// caller-supplied [`sqlx_core::pool::PoolOptions`] instead of the
        /// default (10 connections) -- specifically for
        /// `migrations_complete_under_a_pool_of_size_one`, which needs a
        /// pool built with `max_connections(1)` to reproduce bug 3.
        async fn create_with_pool_options(
            database_url: &str,
            options: sqlx_core::pool::PoolOptions<sqlx_postgres::Postgres>,
        ) -> Self {
            let admin_pool = PgPool::connect(&admin_url(database_url))
                .await
                .expect("connect to the admin/maintenance database");
            let db_name = format!("cfw022_migtest_{}", uuid::Uuid::new_v4().simple());
            sqlx_core::raw_sql::raw_sql(&format!("CREATE DATABASE {db_name}"))
                .execute(&admin_pool)
                .await
                .expect("CREATE DATABASE");
            let pool = options
                .connect(&with_db_name(database_url, &db_name))
                .await
                .expect("connect to the fresh database");
            Self {
                admin_pool,
                db_name,
                pool,
            }
        }
    }

    impl Drop for FreshDatabase {
        fn drop(&mut self) {
            let admin_pool = self.admin_pool.clone();
            let db_name = self.db_name.clone();
            // `Drop` can't be `async`; spawn the cleanup rather than block a
            // runtime thread inside a destructor. Best-effort: a leaked
            // throwaway test database is harmless clutter, not a defect.
            tokio::spawn(async move {
                let sql = format!("DROP DATABASE IF EXISTS {db_name} WITH (FORCE)");
                if let Err(e) = sqlx_core::raw_sql::raw_sql(&sql).execute(&admin_pool).await {
                    eprintln!("warning: failed to drop throwaway test database {db_name}: {e}");
                }
            });
        }
    }

    fn database_url_or_skip() -> Option<String> {
        match std::env::var("DATABASE_URL") {
            Ok(u) => Some(u),
            Err(_) => {
                eprintln!(
                    "skipping migration-runner live tests: DATABASE_URL is not set \
                     (same graceful-skip contract as the Postgres conformance suite)"
                );
                None
            }
        }
    }

    /// `current_version` now takes an already-checked-out `&mut PgConnection`
    /// (bug 3's fix), not a `&PgPool` — this is the test-only convenience
    /// that acquires one for the tests below, which only ever want the
    /// resulting version number, not the connection itself.
    async fn current_version_of(pool: &PgPool) -> i64 {
        let mut conn = pool.acquire().await.expect("acquire a connection");
        current_version(&mut conn)
            .await
            .expect("current_version must succeed")
    }

    #[tokio::test]
    async fn fresh_database_bootstraps_from_version_zero_and_creates_every_table() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let db = FreshDatabase::create(&url).await;

        // Before migrating: `current_version` must see "no schema_migrations
        // table" as version 0, not an error.
        assert_eq!(current_version_of(&db.pool).await, 0);

        run_migrations(&db.pool)
            .await
            .expect("migrations must apply cleanly to a fresh database");

        // Every table from 001_initial.sql now exists and is queryable.
        for table in ["manifest", "policy", "assignment", "user_config"] {
            sqlx_core::query::query::<sqlx_postgres::Postgres>(&format!(
                "SELECT COUNT(*) FROM {table}"
            ))
            .fetch_one(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("table {table} must exist and be queryable: {e}"));
        }
        assert_eq!(current_version_of(&db.pool).await, 1);
    }

    #[tokio::test]
    async fn running_migrations_twice_against_the_same_database_is_a_no_op() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let db = FreshDatabase::create(&url).await;
        run_migrations(&db.pool).await.unwrap();
        run_migrations(&db.pool)
            .await
            .expect("second run must be a no-op, not an error");
        assert_eq!(current_version_of(&db.pool).await, 1);
    }

    #[tokio::test]
    async fn a_database_recorded_ahead_of_this_binarys_known_migrations_is_refused() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let db = FreshDatabase::create(&url).await;
        run_migrations(&db.pool).await.unwrap();

        sqlx_core::query::query::<sqlx_postgres::Postgres>(
            "INSERT INTO schema_migrations (version) VALUES ($1)",
        )
        .bind(999_i64)
        .execute(&db.pool)
        .await
        .unwrap();

        let err = run_migrations(&db.pool).await.unwrap_err();
        assert!(
            matches!(
                err,
                MigrationError::DatabaseAheadOfBinary {
                    found: 999,
                    known: 1
                }
            ),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn a_gap_in_the_migration_list_is_refused() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let db = FreshDatabase::create(&url).await;

        // A synthetic two-entry list with a gap at version 2 -- exercises
        // `MigrationError::Gap`, which `MIGRATIONS` itself (a single real
        // migration today) cannot reach.
        let synthetic: &[(i64, &str)] = &[
            (1, include_str!("migrations/001_initial.sql")),
            (3, "SELECT 1;"),
        ];
        let mut conn = db.pool.acquire().await.unwrap();
        let err = run_migrations_with(&mut conn, synthetic).await.unwrap_err();
        assert!(
            matches!(err, MigrationError::Gap { from: 1, to: 3 }),
            "got {err:?}"
        );

        // The version-1 migration that *did* apply before the gap was hit
        // must have committed -- a half-applied migration set is exactly
        // what "each migration in its own transaction" is meant to avoid
        // corrupting further.
        assert_eq!(current_version(&mut conn).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_migration_with_invalid_sql_fails_loudly_and_does_not_advance_the_version() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let db = FreshDatabase::create(&url).await;

        let broken: &[(i64, &str)] = &[(1, "THIS IS NOT VALID SQL AT ALL;")];
        let mut conn = db.pool.acquire().await.unwrap();
        let err = run_migrations_with(&mut conn, broken).await.unwrap_err();
        assert!(
            matches!(err, MigrationError::Failed { version: 1, .. }),
            "got {err:?}"
        );

        // The failed migration's transaction must have rolled back --
        // `schema_migrations` (which the broken SQL never got a chance to
        // create) still doesn't exist, so `current_version` reports 0.
        assert_eq!(current_version(&mut conn).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_migration_that_conflicts_with_its_own_bookkeeping_insert_fails_loudly() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let db = FreshDatabase::create(&url).await;

        // This synthetic migration 1 creates `schema_migrations` itself
        // *and* pre-inserts its own row for version 1 -- so the runner's
        // own hard-coded bookkeeping `INSERT INTO schema_migrations
        // (version) VALUES ($1)` afterward collides on the primary key,
        // exercising the `Failed` branch around that specific statement
        // rather than the raw-SQL-execution one above.
        let conflicting: &[(i64, &str)] = &[(
            1,
            "CREATE TABLE schema_migrations (version BIGINT PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT now()); \
             INSERT INTO schema_migrations (version) VALUES (1);",
        )];
        let mut conn = db.pool.acquire().await.unwrap();
        let err = run_migrations_with(&mut conn, conflicting)
            .await
            .unwrap_err();
        assert!(
            matches!(err, MigrationError::Failed { version: 1, .. }),
            "got {err:?}"
        );
    }

    /// Bug 3's direct reproduction: a pool constructed with
    /// `max_connections(1)` must still be sufficient for `run_migrations`
    /// to complete. Before the fix, `run_migrations` held the pool's one
    /// connection for the advisory lock and then tried to acquire a
    /// *second* connection (via `current_version(pool)` / `pool.begin()`
    /// inside `run_migrations_holding_lock`) from the same exhausted pool —
    /// a self-inflicted deadlock that only resolves (with an error, not a
    /// true infinite hang) once the pool's `acquire_timeout` elapses. This
    /// test uses a short `acquire_timeout` specifically so that a
    /// regression here fails fast instead of stalling the suite.
    #[tokio::test]
    async fn migrations_complete_under_a_pool_of_size_one() {
        let Some(url) = database_url_or_skip() else {
            return;
        };
        let options = sqlx_core::pool::PoolOptions::<sqlx_postgres::Postgres>::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(5));
        let db = FreshDatabase::create_with_pool_options(&url, options).await;

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(20), run_migrations(&db.pool))
                .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                panic!("migrations must complete under a pool of size 1, got an error instead: {e}")
            }
            Err(_) => panic!(
                "migrations timed out (deadlocked) under a pool of size 1 -- this is exactly \
                 bug 3's self-inflicted deadlock"
            ),
        }
        assert_eq!(current_version_of(&db.pool).await, 1);
    }

    #[test]
    fn admin_url_and_with_db_name_swap_only_the_final_path_segment() {
        let url = "postgresql://postgres:postgres@localhost:5432/postgres";
        assert_eq!(admin_url(url), url);
        assert_eq!(
            with_db_name(url, "my_test_db"),
            "postgresql://postgres:postgres@localhost:5432/my_test_db"
        );
    }
}
