//! Error types for the config service (spec 022).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// A storage-backend failure, common to both [`super::store::PolicyStore`]
/// and [`super::store::UserConfigStore`]. Deliberately backend-agnostic —
/// neither trait's signature ever names `sqlx` or any bundle-format detail,
/// so a caller (the router, startup validation) never needs to know which
/// implementation produced it.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("config service storage backend error: {0}")]
    Backend(String),
    #[error("stored document for app '{app}' failed to parse: {message}")]
    Corrupt { app: String, message: String },
}

impl StoreError {
    pub fn backend(msg: impl std::fmt::Display) -> Self {
        Self::Backend(msg.to_string())
    }
}

/// Failure from a validated [`super::identity::CallerIdentity`] check — the
/// router maps every variant to `401 Unauthorized`. Kept as its own enum
/// (rather than a bare string) so a future caller could distinguish "no
/// credential offered" from "credential rejected" without a wire-format
/// change; today the HTTP response is identical either way; see
/// [`ConfigServiceError::into_response`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigServiceError {
    #[error("no Authorization header presented")]
    MissingCredential,
    #[error("credential rejected: {0}")]
    InvalidCredential(String),
}

impl IntoResponse for ConfigServiceError {
    fn into_response(self) -> Response {
        (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", "Bearer")],
            axum::Json(json!({ "error": self.to_string() })),
        )
            .into_response()
    }
}

/// Why writing a roaming user document failed — the router maps
/// [`Self::Conflict`] to `412 Precondition Failed` and every other variant to
/// `400 Bad Request` (validation) or `500` (storage), per
/// [`super::router`]'s handler.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UserConfigWriteError {
    /// The caller's `If-Match` version no longer matches the stored
    /// document's current version — someone else (or another of the
    /// caller's own devices) wrote it first.
    #[error("stored document is at version {current}, not the expected {expected}")]
    Conflict { current: u64, expected: u64 },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Why a stored policy failed manifest-conformance validation — every
/// variant maps 1:1 onto a
/// [`crate::config::resolution::WarningReason`] variant (the *client-side*
/// defence-in-depth drop reason) or an inheritance-integrity failure this
/// slice adds on top. Produced by [`super::validate::validate_stored_policy`],
/// which calls the resolver's own drop-reason rules rather than
/// re-implementing them — see that module's docs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyValidationError {
    #[error("app '{app}' profile '{profile}': field '{path}' does not exist in the manifest")]
    UnknownField {
        app: String,
        profile: String,
        path: String,
    },
    #[error("app '{app}' profile '{profile}': field '{path}' has a value of the wrong type")]
    TypeMismatch {
        app: String,
        profile: String,
        path: String,
    },
    #[error("app '{app}' profile '{profile}': field '{path}' is local_only and cannot appear in a policy")]
    LocalOnly {
        app: String,
        profile: String,
        path: String,
    },
    #[error("app '{app}' profile '{profile}': field '{path}' is not manageable and cannot appear in a policy")]
    NotManageable {
        app: String,
        profile: String,
        path: String,
    },
    #[error(
        "app '{app}' profile '{profile}': field '{path}' is secret and cannot appear in a policy"
    )]
    Secret {
        app: String,
        profile: String,
        path: String,
    },
    #[error(
        "app '{app}' profile '{profile}': org-scoped field '{path}' cannot appear in recommended"
    )]
    OrgScopeInRecommended {
        app: String,
        profile: String,
        path: String,
    },
    #[error("app '{app}' profile '{profile}': field '{path}' is enforceable=false and cannot appear in enforced")]
    NotEnforceable {
        app: String,
        profile: String,
        path: String,
    },
    /// `field.constraints` (`min`/`max`/`allowed_values` — spec 021 user
    /// story 6) rejected the stored value. Spec 024 review, Fix 1: these
    /// were previously carried but never enforced anywhere server-side (only
    /// rendered into a JSON Schema document for a UI by
    /// `crate::config::manifest::json_schema`) — an admin write or a
    /// roaming user-config write could land a value wildly outside a
    /// field's declared bounds. `detail` names which bound was violated and
    /// by what value, since (unlike this enum's other variants) that detail
    /// can't be reconstructed from `app`/`profile`/`path` alone.
    #[error("app '{app}' profile '{profile}': field '{path}' violates its declared constraint: {detail}")]
    ConstraintViolation {
        app: String,
        profile: String,
        path: String,
        detail: String,
    },
    #[error("app '{app}' has a policy for profile '{profile}' but no manifest is stored for it")]
    MissingManifest { app: String, profile: String },
    #[error("app '{app}' profile '{profile}' names parent '{parent}', which has no stored policy")]
    MissingParent {
        app: String,
        profile: String,
        parent: String,
    },
    #[error("app '{app}' profile '{profile}' is part of an inheritance cycle")]
    InheritanceCycle { app: String, profile: String },
    /// An assignment rule (ordinary or the terminal `Default` row alike —
    /// see [`super::types::RuleOperator::Default`]) names a `profile` with
    /// no stored policy for this app. Bug 2's fix: previously this passed
    /// startup validation silently and only surfaced later as a `404
    /// unmanaged` for any identity that rule would otherwise have matched.
    #[error(
        "app '{app}' assignment rule (ord {ord}) targets profile '{profile}', which has no stored policy for this app"
    )]
    AssignmentRuleMissingProfile {
        app: String,
        ord: i64,
        profile: String,
    },
    /// A `Default`-operator assignment row exists for this app but is not
    /// the last-ordered row (see [`super::types::RuleOperator::Default`]'s
    /// doc comment for why this matters — a `Default` row anywhere but last
    /// silently preempts every rule ordered after it, since it matches
    /// unconditionally and evaluation is first-match-wins by ascending
    /// `ord`). Bug 4's fix.
    #[error(
        "app '{app}': default-profile assignment rule (ord {ord}) is not the last-ordered rule for this app (max ord is {max_ord}); it would silently preempt every rule ordered after it"
    )]
    DefaultRuleNotLast { app: String, ord: i64, max_ord: i64 },
}

/// Failure walking a stored policy's `parent_profile` chain at **read**
/// time (the defence-in-depth counterpart to
/// [`PolicyValidationError::MissingParent`] / `InheritanceCycle`, which are
/// the **startup**-time checks over the same rule). Startup validation
/// should make [`Self::MissingParent`]/[`Self::Cycle`] unreachable in a
/// correctly-validated deployment; the router still treats them as an
/// internal error (`500`) rather than panicking, per spec 022's "refuse
/// rather than infinite-loop on a cycle that somehow got in."
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InheritanceError {
    /// `start` itself has no stored policy — this is the ordinary "this
    /// profile has no policy document" case (the caller maps it to `404`,
    /// not `500`: it is not a data-integrity failure, just nothing to
    /// serve).
    #[error("no stored policy for profile '{profile}'")]
    ProfileNotFound { profile: String },
    /// A non-root ancestor in the chain names a parent with no stored
    /// policy at all. Unlike [`Self::ProfileNotFound`], this is reached only
    /// after at least one policy in the chain resolved successfully, so it
    /// signals a genuinely broken chain rather than "nothing here yet."
    #[error("profile '{child}' names parent '{parent}', which has no stored policy")]
    MissingParent { child: String, parent: String },
    #[error("profile '{profile}' is part of an inheritance cycle")]
    Cycle { profile: String },
}

/// Why an administrative write ([`super::admin_store::PolicyAdminStore`],
/// spec 023) failed. The router maps [`Self::Conflict`] to `412 Precondition
/// Failed` (the same optimistic-concurrency outcome
/// [`UserConfigWriteError::Conflict`] already maps to `412` for the
/// device-facing write path — spec 023 is explicit this is "one mechanism,
/// not two"), [`Self::Validation`] to `400 Bad Request` (listing every
/// error, not just the first), and [`Self::Store`] to `500`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AdminWriteError {
    /// The caller's `expected_version` (from `If-Match`, or `0` to mean
    /// "must not already exist") no longer matches the stored resource's
    /// current version.
    #[error("stored version is {current}, expected {expected}")]
    Conflict { current: u64, expected: u64 },
    /// Manifest-conformance or inheritance-integrity validation rejected the
    /// candidate document before any storage write was attempted — see
    /// [`super::validate::validate_stored_policy`]/[`super::inherit::resolve_chain`],
    /// which this variant's contents come from unchanged (spec 023: "using
    /// the same validator... not a second copy of the rules").
    #[error("write failed validation: {0:?}")]
    Validation(Vec<PolicyValidationError>),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The result of [`super::validate::validate_all`] — every conformance
/// failure found across every stored policy, surfaced together rather than
/// stopping at the first one, so a deployment operator can fix a broken
/// bundle/database in one pass instead of one failure at a time.
///
/// Implements `Display`/`Error` by hand (rather than via
/// `#[derive(thiserror::Error)]`) because the message needs to join a
/// `Vec` field's `Display` output, which does not fit thiserror's
/// single-expression `#[error("...")]` shorthand cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupValidationError(pub Vec<PolicyValidationError>);

impl std::fmt::Display for StartupValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "config service startup validation failed with {} error(s)",
            self.0.len()
        )?;
        for e in &self.0 {
            write!(f, "\n  - {e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for StartupValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_validation_error_display_lists_the_count_and_every_error() {
        let err = StartupValidationError(vec![
            PolicyValidationError::UnknownField {
                app: "myapp".to_string(),
                profile: "developers".to_string(),
                path: "ghost".to_string(),
            },
            PolicyValidationError::Secret {
                app: "myapp".to_string(),
                profile: "developers".to_string(),
                path: "api_key".to_string(),
            },
        ]);
        let text = err.to_string();
        assert!(text.contains("2 error(s)"));
        assert!(text.contains("ghost"));
        assert!(text.contains("api_key"));
    }

    #[test]
    fn startup_validation_error_display_with_zero_errors() {
        let err = StartupValidationError(vec![]);
        assert!(err.to_string().contains("0 error(s)"));
    }

    #[test]
    fn admin_write_error_conflict_displays_both_versions() {
        let err = AdminWriteError::Conflict {
            current: 5,
            expected: 3,
        };
        let text = err.to_string();
        assert!(text.contains('5'));
        assert!(text.contains('3'));
    }

    #[test]
    fn admin_write_error_validation_displays_the_errors() {
        let err = AdminWriteError::Validation(vec![PolicyValidationError::UnknownField {
            app: "myapp".to_string(),
            profile: "developers".to_string(),
            path: "ghost".to_string(),
        }]);
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn admin_write_error_store_is_transparent() {
        let err: AdminWriteError = StoreError::backend("boom").into();
        assert!(matches!(err, AdminWriteError::Store(_)));
        assert!(err.to_string().contains("boom"));
    }
}
