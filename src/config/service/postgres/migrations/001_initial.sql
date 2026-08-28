-- Migration 1: initial schema for the config service (spec 022).
--
-- `max_cache_age_secs` and `stale_action` on `policy` are additions beyond
-- the four bare column lists spec 022's task text names verbatim
-- (`policy(app, profile, enforced, recommended, parent_profile, version)`)
-- -- see `crate::config::service::types::StoredPolicy`'s doc comment for why
-- they are necessary: the wire `Policy` document (spec 021) requires both,
-- and spec 021 ties them to "the policy itself," i.e. per (app, profile).
--
-- `assignment.operator` includes a fourth value, `'default'`, beyond the
-- three spec 022 names (`equals`/`contains`/`exists`) -- see
-- `RuleOperator::Default`'s doc comment for why "an optional default
-- profile" is represented as a terminal row in this table rather than a
-- fifth table or an extra nullable column.

CREATE TABLE schema_migrations (
    version    BIGINT PRIMARY KEY,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE manifest (
    app     TEXT PRIMARY KEY,
    doc     JSONB NOT NULL,
    version BIGINT NOT NULL
);

CREATE TABLE policy (
    app                 TEXT NOT NULL,
    profile             TEXT NOT NULL,
    enforced            JSONB NOT NULL DEFAULT '{}'::jsonb,
    recommended         JSONB NOT NULL DEFAULT '{}'::jsonb,
    parent_profile      TEXT,
    max_cache_age_secs  BIGINT NOT NULL DEFAULT 3600,
    stale_action        TEXT NOT NULL DEFAULT 'warn',
    version             BIGINT NOT NULL,
    PRIMARY KEY (app, profile)
);

CREATE TABLE assignment (
    app        TEXT NOT NULL,
    ord        BIGINT NOT NULL,
    claim_path TEXT NOT NULL,
    operator   TEXT NOT NULL,
    value      JSONB,
    profile    TEXT NOT NULL,
    PRIMARY KEY (app, ord)
);

CREATE TABLE user_config (
    app     TEXT NOT NULL,
    subject TEXT NOT NULL,
    doc     JSONB NOT NULL DEFAULT '{}'::jsonb,
    version BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (app, subject)
);
