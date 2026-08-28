//! The layered configuration resolver (spec 021, "Resolution order"): folds
//! a [`crate::config::manifest::ConfigManifest`] and six layers of values
//! into resolved values plus [`Provenance`], with `enforced` applied last as
//! a veto pass rather than a seventh layer.
//!
//! Works entirely against `ConfigManifest` + `serde_json::Value` — never
//! against the Rust type a `#[derive(ConfigManifest)]` struct was applied
//! to, so a hand-authored (non-Rust) manifest resolves through the identical
//! code path (spec 021, "Manifest schema is data, not Rust types, at the
//! consumption boundary").

mod flatten;
mod provenance;
#[allow(clippy::module_inception)]
mod resolver;

pub use flatten::{flatten_to_paths, unflatten_from_paths};
pub use provenance::{Layer, Provenance};
pub use resolver::{
    resolve, ResolutionInput, ResolutionWarning, Resolved, ResolvedEntry, WarningReason,
};
