-- Migration 2: administrative API and mutation log for the config service
-- (spec 023).
--
-- `mutation_log` deliberately has NO foreign key to `manifest`, `policy`, or
-- `assignment` — spec 023 user story 23 requires the log to survive deletion
-- of the rows it describes ("the log retained independently of the policy it
-- describes, so that deleting a profile does not erase the record of what it
-- once enforced"). A FK (cascade OR restrict) would either delete history
-- alongside the row it documents, or block deleting the row at all — both
-- wrong. `app`/`profile` here are plain, unconstrained text columns, not
-- references.
--
-- `assignment_set` exists purely to give the `assignment` table (migration
-- 001) a version counter: `assignment(app, ord, ...)` has no per-app version
-- column at all, so `/v1/admin/assignments/{app}` has nothing to key
-- `If-Match`/`ETag` off without it.

CREATE TABLE mutation_log (
    id                 BIGSERIAL PRIMARY KEY,
    app                TEXT NOT NULL,
    profile            TEXT,               -- NULL for manifest-level and assignments-level mutations
    kind               TEXT NOT NULL,      -- 'manifest_put' | 'policy_put' | 'policy_patch' | 'policy_restore' | 'assignments_put' | 'import'
    actor              TEXT NOT NULL,
    occurred_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    submitted          JSONB NOT NULL,     -- exactly what the caller's request body contained
    resulting_document JSONB NOT NULL,     -- full resulting state snapshot after the change
    resulting_version  BIGINT NOT NULL
);

CREATE INDEX mutation_log_app_profile_version_idx ON mutation_log (app, profile, resulting_version);

CREATE TABLE assignment_set (
    app     TEXT PRIMARY KEY,
    version BIGINT NOT NULL DEFAULT 0
);
