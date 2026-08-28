//! [`PolicyAdminStore`]: the write side of the config service (spec 023),
//! kept as its own trait rather than added to [`super::store::PolicyStore`]
//! — that trait's own docs are explicit it is read-only ("Read-only at the
//! API surface in this slice"), and [`super::fs_store::FsPolicyStore`]
//! deliberately has no write path at all (it exists for tests and local
//! development, loaded once into memory). Only
//! [`super::postgres::PgPolicyStore`] implements this trait.
//!
//! Every method's own doc comment states the transactional guarantee it
//! must uphold: the state write and its `mutation_log` row land in the same
//! Postgres transaction, so a stored change without a record is never
//! representable (spec 023's central, non-negotiable correctness property).

use super::error::{AdminWriteError, StoreError};
use super::fs_store::FsPolicyStore;
use super::types::{AssignmentRule, MutationKind};
use crate::config::StaleAction;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value};

/// The mutable fields of a stored policy — everything [`super::types::StoredPolicy`]
/// carries except `app`/`profile` (identified separately, by the method's own
/// parameters) and `version` (server-assigned on every write, never supplied
/// by a caller).
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyWrite {
    pub enforced: Map<String, Value>,
    pub recommended: Map<String, Value>,
    pub parent_profile: Option<String>,
    pub max_cache_age_secs: u64,
    pub stale_action: StaleAction,
}

/// One row read back from `mutation_log` (spec 023's append-only change
/// record).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MutationLogEntry {
    pub id: i64,
    pub app: String,
    /// `None` for manifest-level and assignments-level mutations.
    pub profile: Option<String>,
    pub kind: MutationKind,
    pub actor: String,
    pub occurred_at: DateTime<Utc>,
    /// Exactly what the caller's request body contained — for
    /// [`MutationKind::PolicyPatch`] this is the raw merge-patch document,
    /// not the merged result; for [`MutationKind::PolicyRestore`] it is
    /// `{"restore_from_version": N}`, not the restored document itself.
    pub submitted: Value,
    /// The full resulting state snapshot after the change. A convenience
    /// snapshot for restore/audit, never the source of truth — the `policy`/
    /// `manifest`/`assignment` tables remain authoritative.
    pub resulting_document: Value,
    pub resulting_version: u64,
}

/// A compact tally of what an [`PolicyAdminStore::import_bundle`] call
/// wrote — not a per-item manifest, since `mutation_log`'s own `import`-kind
/// row (whose `resulting_document` is this summary, serialized) is not
/// expected to support [`PolicyAdminStore::put_policy`]-style restore the
/// way `policy_put`/`policy_patch`/`policy_restore` rows are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ImportSummary {
    pub manifests: usize,
    pub policies: usize,
    pub assignment_rules: usize,
}

/// Administrative write access to an organisation's target state, plus its
/// append-only change record (spec 023). Every write is optimistic-
/// concurrency-checked (`expected_version`, the same numeric convention the
/// existing `If-Match`/`ETag` device-facing write path already uses) except
/// [`Self::import_bundle`], which is a bulk seed/backup operation, not a
/// single-resource update — see that method's own docs.
#[async_trait]
pub trait PolicyAdminStore: Send + Sync {
    /// Replace `app`'s manifest wholesale. `expected_version: 0` means "no
    /// manifest must currently be stored for `app`" — the same convention
    /// [`super::store::UserConfigStore::put`] already uses for "create a
    /// brand-new document." Returns the new version on success.
    async fn put_manifest(
        &self,
        app: &str,
        doc: crate::config::manifest::ConfigManifest,
        actor: &str,
        expected_version: u64,
    ) -> Result<u64, AdminWriteError>;

    /// Replace (or create) the stored policy for `(app, profile)` with
    /// `policy`, appending one `mutation_log` row of the given `kind` in the
    /// same transaction. `submitted` is exactly what goes into that row's
    /// `submitted` column — the caller (the HTTP handler) decides what that
    /// is, since it differs by `kind`: the full PUT body for
    /// [`MutationKind::PolicyPut`], the raw (unmerged) PATCH body for
    /// [`MutationKind::PolicyPatch`], and `{"restore_from_version": N}` for
    /// [`MutationKind::PolicyRestore`] — never the resulting document
    /// itself, which is instead recorded separately as
    /// [`MutationLogEntry::resulting_document`]. This is a deliberate
    /// addition beyond spec 023's own sketch of this method's signature,
    /// which named no `submitted` parameter at all; without one, this
    /// single method could not honour "the record's `submitted` column is
    /// what the caller actually sent" for all three kinds at once (see this
    /// slice's PR description for the full rationale).
    #[allow(clippy::too_many_arguments)]
    async fn put_policy(
        &self,
        app: &str,
        profile: &str,
        policy: PolicyWrite,
        kind: MutationKind,
        submitted: Value,
        actor: &str,
        expected_version: u64,
    ) -> Result<u64, AdminWriteError>;

    /// The current version of `app`'s assignment-rule set — `0` if none has
    /// ever been written (mirroring [`super::store::UserConfigStore::get`]'s
    /// "no document yet is version 0" convention), which is what
    /// `GET /v1/admin/assignments/{app}` reports as its `ETag` and what a
    /// client's first `PUT` supplies as `expected_version`.
    async fn assignment_rules_version(&self, app: &str) -> Result<u64, StoreError>;

    /// Replace `app`'s entire ordered assignment-rule set with `rules` (the
    /// server assigns `ord` from each rule's position in `rules`, ignoring
    /// whatever the caller may have set), appending one `mutation_log` row
    /// (`kind: `[`MutationKind::AssignmentsPut`]`, `profile: None`).
    async fn put_assignment_rules(
        &self,
        app: &str,
        rules: Vec<AssignmentRule>,
        actor: &str,
        expected_version: u64,
    ) -> Result<u64, AdminWriteError>;

    /// Every `mutation_log` row for `(app, profile)`, ordered ascending by
    /// `resulting_version`. Deliberately survives deletion of the
    /// `(app, profile)` policy row itself — see the `002_admin_mutation_log.sql`
    /// migration's own comment on why there is no foreign key.
    async fn policy_history(
        &self,
        app: &str,
        profile: &str,
    ) -> Result<Vec<MutationLogEntry>, StoreError>;

    /// Validate and store an entire bundle (spec 023, "Export/Import"):
    /// every manifest, policy, and assignment-rule set `bundle` contains is
    /// validated against the bundle's *own* contents (never merged with
    /// whatever is already stored) before anything is written; if any
    /// validation fails anywhere in the bundle, nothing is stored. On
    /// success, every write plus exactly one `mutation_log` row per app
    /// (`kind: `[`MutationKind::Import`]) lands in one transaction.
    ///
    /// Unlike every other method here, this does **not** take an
    /// `expected_version` — import is a bulk seed/backup operation (spec 023
    /// frames it as "seeding a new environment" / "import into an empty
    /// deployment"), not a single-resource optimistic-concurrency write, so
    /// there is no one prior version for a caller to have cached and
    /// compared against.
    async fn import_bundle(
        &self,
        bundle: &FsPolicyStore,
        actor: &str,
    ) -> Result<ImportSummary, AdminWriteError>;
}
