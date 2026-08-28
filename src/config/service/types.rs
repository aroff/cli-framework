//! Server-side storage types for the config service (spec 022).
//!
//! These are deliberately **not** [`crate::config::Policy`] — that type is
//! the *wire* shape a client receives after the server has already resolved
//! a profile and flattened inheritance. The types here are what an
//! organisation actually stores: one row per application (the manifest),
//! one row per (application, profile) pair (a policy, which may still name a
//! parent to inherit from), one row per ordered assignment rule, and one row
//! per (application, subject) roaming document. [`crate::config::service::router`]
//! is what turns a [`StoredPolicy`] plus its inheritance chain into a
//! [`crate::config::Policy`] on the wire.

use crate::config::manifest::ConfigManifest;
use crate::config::StaleAction;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// An application's manifest as stored by the service — the same
/// [`ConfigManifest`] document the application itself published, plus a
/// version used for diagnostics (this slice never writes it, so it is always
/// whatever the seeding path stamped).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredManifest {
    pub app: String,
    pub doc: ConfigManifest,
    pub version: u64,
}

/// One stored policy: an application + profile pair, the two trees an
/// organisation authored for it, an optional single parent to inherit from,
/// and the cache-control metadata a resolved [`crate::config::Policy`] wire
/// document carries.
///
/// `max_cache_age_secs`/`stale_action` are **not** among the columns spec
/// 022's task text lists verbatim for the `policy` table (`app, profile,
/// enforced, recommended, parent_profile, version`) — they were added as a
/// necessary extension, not an oversight: [`crate::config::Policy`] (spec
/// 021's wire contract, ADR 0072 user story 5 — "I want the maximum cache
/// age and stale behaviour to come from the policy itself") requires both
/// fields, and spec 021 ties them to "the policy itself," i.e. per
/// (app, profile), not a server-wide constant. See the module docs on
/// `postgres::migrations` for the exact schema and the report this slice's
/// author filed for this specific deviation.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPolicy {
    pub app: String,
    pub profile: String,
    pub enforced: Map<String, Value>,
    pub recommended: Map<String, Value>,
    pub parent_profile: Option<String>,
    pub max_cache_age_secs: u64,
    pub stale_action: StaleAction,
    pub version: u64,
}

/// The three assignment-rule operators spec 022 defines, plus one this
/// implementation adds to represent "optional default profile" within the
/// exact four-table schema spec 022 specifies (`assignment(app, ord,
/// claim_path, operator, value, profile)` has no separate "default profile"
/// column or table) — see [`RuleOperator::Default`]'s own docs for the
/// reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleOperator {
    /// Exact match against a scalar claim.
    Equals,
    /// The claim is an array and one element equals `value`.
    Contains,
    /// The claim path resolves to anything at all; `value` is ignored.
    Exists,
    /// Unconditionally matches. **Not** one of spec 022's three named
    /// operators — a judgment call (documented in this slice's report) to
    /// represent "an optional default profile" as an ordinary terminal row
    /// in the `assignment` table's exact given schema, rather than adding a
    /// fifth table or a nullable column spec 022's table list didn't name.
    /// A stored default rule's `claim_path` is conventionally empty and its
    /// `value` is `None`; both are ignored by evaluation.
    ///
    /// **Load-bearing invariant, enforced by validation, not by
    /// construction**: because `Default` unconditionally matches
    /// (`assignment::rule_matches`) and rules are evaluated in ascending
    /// `ord` order, first-match-wins (`assignment::resolve_profile`), a
    /// `Default` row that is not the **last**-ordered row for its app
    /// silently preempts every rule ordered after it — an identity that
    /// should have matched a specific rule instead falls through to the
    /// default, with no error anywhere. Nothing in this type (or in the
    /// `assignment` table's schema) prevents a `Default` row from being
    /// stored out of order; the type system alone cannot express "must sort
    /// last." [`super::validate::validate_all`] is what actually enforces
    /// this, by rejecting at startup any app whose `Default` row (if one
    /// exists) does not have the maximum `ord` among that app's rules.
    /// Spec 023 reuses this exact rule-evaluation mechanism for its
    /// administrative role gate — it inherits this same invariant, and the
    /// same enforcement point, rather than needing to rediscover the gap.
    Default,
}

impl RuleOperator {
    /// The canonical wire/storage string for this operator — used by
    /// [`super::postgres`]'s `operator` TEXT column and by
    /// [`Self::parse_wire_str`]'s inverse. The single source of truth for
    /// this mapping, so the Postgres store and the bundle-directory store
    /// can't drift apart on what a given string means.
    pub fn wire_str(self) -> &'static str {
        match self {
            Self::Equals => "equals",
            Self::Contains => "contains",
            Self::Exists => "exists",
            Self::Default => "default",
        }
    }

    /// The inverse of [`Self::wire_str`]. `None` for anything else,
    /// including case variants — callers map that to their own
    /// backend-appropriate error.
    pub fn parse_wire_str(s: &str) -> Option<Self> {
        match s {
            "equals" => Some(Self::Equals),
            "contains" => Some(Self::Contains),
            "exists" => Some(Self::Exists),
            "default" => Some(Self::Default),
            _ => None,
        }
    }
}

/// One ordered assignment rule for one application.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentRule {
    pub app: String,
    /// Evaluation order — ascending, first match wins. Not assumed to be
    /// contiguous or zero-based; only relative order matters.
    pub ord: i64,
    pub claim_path: String,
    pub operator: RuleOperator,
    pub value: Option<Value>,
    pub profile: String,
}

/// What kind of administrative write produced one [`mutation_log`] row
/// (spec 023). Follows the exact `wire_str()`/`parse_wire_str()` convention
/// [`RuleOperator`] already established, even though the underlying
/// `mutation_log.kind` column is a bare `TEXT` — so the Postgres store and
/// (if a future bundle-export format ever wants to round-trip history) any
/// other reader share one mapping rather than each inventing their own
/// string literals.
///
/// [`mutation_log`]: super::postgres — see that module's `002_admin_mutation_log.sql`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationKind {
    /// `PUT /v1/admin/manifest/{app}`.
    ManifestPut,
    /// `PUT /v1/admin/policy/{app}/{profile}`.
    PolicyPut,
    /// `PATCH /v1/admin/policy/{app}/{profile}`.
    PolicyPatch,
    /// `POST /v1/admin/policy/{app}/{profile}/history/{version}/restore`.
    PolicyRestore,
    /// `PUT /v1/admin/assignments/{app}`.
    AssignmentsPut,
    /// `POST /v1/admin/import`.
    Import,
}

impl MutationKind {
    /// The canonical wire/storage string for this kind — used by
    /// `mutation_log.kind` and by [`Self::parse_wire_str`]'s inverse.
    pub fn wire_str(self) -> &'static str {
        match self {
            Self::ManifestPut => "manifest_put",
            Self::PolicyPut => "policy_put",
            Self::PolicyPatch => "policy_patch",
            Self::PolicyRestore => "policy_restore",
            Self::AssignmentsPut => "assignments_put",
            Self::Import => "import",
        }
    }

    /// The inverse of [`Self::wire_str`]. `None` for anything else.
    pub fn parse_wire_str(s: &str) -> Option<Self> {
        match s {
            "manifest_put" => Some(Self::ManifestPut),
            "policy_put" => Some(Self::PolicyPut),
            "policy_patch" => Some(Self::PolicyPatch),
            "policy_restore" => Some(Self::PolicyRestore),
            "assignments_put" => Some(Self::AssignmentsPut),
            "import" => Some(Self::Import),
            _ => None,
        }
    }
}

/// A stored roaming user-scoped document for one (application, subject) pair.
///
/// `version` starts at `0` for a subject with no document yet (never `None`
/// / never absent) — see [`crate::config::service::store::UserConfigStore`]'s
/// docs for why "no document yet" is modelled as version `0` with an empty
/// `doc` rather than an `Option`.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredUserConfig {
    pub app: String,
    pub subject: String,
    pub doc: Map<String, Value>,
    pub version: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_str_and_parse_wire_str_round_trip_for_every_operator() {
        for op in [
            RuleOperator::Equals,
            RuleOperator::Contains,
            RuleOperator::Exists,
            RuleOperator::Default,
        ] {
            assert_eq!(RuleOperator::parse_wire_str(op.wire_str()), Some(op));
        }
    }

    #[test]
    fn parse_wire_str_rejects_anything_else() {
        assert_eq!(RuleOperator::parse_wire_str("startswith"), None);
        assert_eq!(RuleOperator::parse_wire_str(""), None);
        assert_eq!(RuleOperator::parse_wire_str("EQUALS"), None);
    }

    #[test]
    fn mutation_kind_wire_str_and_parse_wire_str_round_trip_for_every_variant() {
        for kind in [
            MutationKind::ManifestPut,
            MutationKind::PolicyPut,
            MutationKind::PolicyPatch,
            MutationKind::PolicyRestore,
            MutationKind::AssignmentsPut,
            MutationKind::Import,
        ] {
            assert_eq!(MutationKind::parse_wire_str(kind.wire_str()), Some(kind));
        }
    }

    #[test]
    fn mutation_kind_parse_wire_str_rejects_anything_else() {
        assert_eq!(MutationKind::parse_wire_str("policy_delete"), None);
        assert_eq!(MutationKind::parse_wire_str(""), None);
        assert_eq!(MutationKind::parse_wire_str("PolicyPut"), None);
    }
}
