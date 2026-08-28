//! [`PolicyStore`] and [`UserConfigStore`]: the storage seam (spec 022),
//! mirroring how `crate::secrets::SecretStore` already separates a trait
//! from its backends.
//!
//! Two [`PolicyStore`] implementations ship: [`super::postgres::PgPolicyStore`]
//! (the real backend) and [`super::fs_store::FsPolicyStore`] (a read-only
//! bundle-directory store for tests and local development, spec 022's
//! "Bundle format"). [`UserConfigStore`] ships
//! [`super::postgres::PgUserConfigStore`] plus
//! [`super::memory_store::InMemoryUserConfigStore`] for tests.
//!
//! Both traits are `async_trait` and `Send + Sync`, so a caller can hold
//! either behind an `Arc<dyn PolicyStore>` / `Arc<dyn UserConfigStore>` and
//! call it concurrently from multiple request handlers — the same
//! requirement `SecretStore` documents on itself.

use super::error::{StoreError, UserConfigWriteError};
use super::types::{AssignmentRule, StoredManifest, StoredPolicy, StoredUserConfig};
use async_trait::async_trait;
use serde_json::{Map, Value};

/// Read access to an organisation's target state: manifests, policies, and
/// assignment rules. Read-only at the API surface in this slice (spec 022 is
/// explicit that administrative writes are PRD 023's job) — rows arrive by
/// seeding, migration, or (for [`super::fs_store::FsPolicyStore`]) the
/// bundle-directory format.
#[async_trait]
pub trait PolicyStore: Send + Sync {
    /// The stored manifest for `app`, or `None` if no manifest has been
    /// published for it.
    async fn manifest(&self, app: &str) -> Result<Option<StoredManifest>, StoreError>;

    /// The stored policy for exactly one `(app, profile)` pair, or `None` if
    /// no policy has been authored for that profile.
    async fn policy(&self, app: &str, profile: &str) -> Result<Option<StoredPolicy>, StoreError>;

    /// Every stored policy for `app`, across every profile — the input
    /// [`super::inherit::resolve_chain`] needs to walk a `parent_profile`
    /// chain, and what startup validation
    /// ([`super::validate::validate_all`]) iterates to check every policy at
    /// once.
    async fn policies_for_app(&self, app: &str) -> Result<Vec<StoredPolicy>, StoreError>;

    /// Every assignment rule for `app`. Implementations are not required to
    /// return them pre-sorted by `ord` — [`super::assignment::resolve_profile`]
    /// sorts defensively — but SHOULD store and return them in a stable
    /// order regardless.
    async fn assignment_rules(&self, app: &str) -> Result<Vec<AssignmentRule>, StoreError>;

    /// Every application name this store knows about (has a manifest and/or
    /// at least one policy for) — what
    /// [`super::validate::validate_all`] iterates at startup.
    async fn apps(&self) -> Result<Vec<String>, StoreError>;
}

/// Read/write access to roaming, user-scoped documents — one per
/// `(app, subject)` pair.
///
/// "No document exists yet" is modelled as a document at **version 0** with
/// an empty body, never as `Option::None` — this is what lets
/// [`Self::put`]'s `expected_version` parameter use the same optimistic-
/// concurrency mechanism uniformly for both the very first write (client
/// supplies `expected_version: 0`, exactly what an initial [`Self::get`]
/// already reported) and every subsequent write, with no special-cased
/// "creating" branch in the router.
#[async_trait]
pub trait UserConfigStore: Send + Sync {
    /// The current document for `(app, subject)`. Never fails with "not
    /// found" — an absent document is version `0` with an empty body.
    async fn get(&self, app: &str, subject: &str) -> Result<StoredUserConfig, StoreError>;

    /// Replace the document for `(app, subject)` with `doc`, conditioned on
    /// the caller's `expected_version` matching the document's current
    /// stored version. Returns the new version on success.
    ///
    /// Implementations MUST perform the compare-and-set atomically (no
    /// window in which two concurrent callers with the same
    /// `expected_version` can both succeed) — see the conformance suite's
    /// `if_match_mismatch_leaves_the_stored_document_unchanged` scenario,
    /// which every implementation is tested against identically.
    async fn put(
        &self,
        app: &str,
        subject: &str,
        doc: Map<String, Value>,
        expected_version: u64,
    ) -> Result<u64, UserConfigWriteError>;
}
