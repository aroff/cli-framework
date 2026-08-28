//! [`ConfigManifest`]: an application's declared configuration surface (spec
//! 021, ADR 0073).
//!
//! A manifest is a plain JSON document — see [`ConfigManifest`] — generated from a
//! config struct's field attributes by `#[derive(ConfigManifest)]` (added to
//! `cli-framework-macros` alongside `#[derive(CommandSpec)]`, gated by the
//! `derive` feature) for Rust applications, and hand-authored identically by
//! non-Rust applications. Every downstream consumer (the resolver in
//! [`crate::config::resolution`], a provenance query, a hypothetical
//! non-Rust settings renderer) reads the JSON document alone — never the
//! Rust type the derive macro was applied to.
//!
//! ```
//! use cli_framework::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
//!
//! let manifest = ConfigManifest::new(
//!     "myapp",
//!     vec![FieldManifest {
//!         key: "greeting".to_string(),
//!         kind: FieldKind::Str,
//!         default: Some(serde_json::json!("hello")),
//!         label: Some("Greeting".to_string()),
//!         description: None,
//!         group: None,
//!         scope: Scope::User,
//!         platforms: vec![],
//!         secret: false,
//!         local_only: false,
//!         protected: false,
//!         manageable: true,
//!         enforceable: true,
//!         restart_required: false,
//!         constraints: None,
//!     }],
//! );
//! assert_eq!(manifest.iter_leaves().len(), 1);
//! ```

mod json_schema;
mod types;

pub use json_schema::to_json_schema;
pub use types::{
    ConfigManifest, FieldConstraints, FieldKind, FieldManifest, LeafField, Scope,
    MANIFEST_SCHEMA_VERSION,
};

/// Implemented by `#[derive(ConfigManifest)]` for a config struct — the Rust
/// entry point that *produces* the JSON document, never a type any consumer
/// of the document needs to know about. Mirrors
/// `cli_framework::command::IntoCommandSpec` in shape and intent.
pub trait IntoConfigManifest {
    fn config_manifest() -> ConfigManifest;
}
