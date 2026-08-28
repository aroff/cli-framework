//! [`Provenance`]: which layer produced a resolved field's value, and
//! whether it is locked (spec 021, ADR 0072).
//!
//! Exposed as a first-class query on [`super::Resolved`] rather than a
//! debugging aid: a settings UI that cannot distinguish "managed by your
//! organisation" from "you chose this" will silently discard user edits.

use serde::{Deserialize, Serialize};

/// The resolution order (spec 021):
/// `defaults -> recommended -> config file -> environment -> flags ->
/// builder overrides -> ENFORCED`.
///
/// `Enforced` is not really "one more layer that happens to be last" — it is
/// a veto pass applied over whatever the other six layers produced. It is
/// listed last here only because [`Provenance::locked`] is defined as "was
/// this the enforced pass," which requires `Enforced` to be a distinguishable
/// value in this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Default,
    Recommended,
    ConfigFile,
    Environment,
    Flags,
    BuilderOverride,
    Enforced,
}

/// Which [`Layer`] produced a resolved field's value, and whether that value
/// is locked (i.e. an organisation's [`crate::config::Policy`] enforces it —
/// see ADR 0072's "Enforced" concept).
///
/// `locked` is not independently settable — it is exactly `layer ==
/// Layer::Enforced`. Kept as an explicit field (rather than a method) because
/// spec 021 describes provenance as "which layer produced the value, and
/// whether it is locked" as two separate pieces of information a renderer
/// asks for, even though the second is a pure function of the first today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub layer: Layer,
    pub locked: bool,
}

impl Provenance {
    pub fn new(layer: Layer) -> Self {
        Self {
            locked: layer == Layer::Enforced,
            layer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_enforced_layer_is_locked() {
        for layer in [
            Layer::Default,
            Layer::Recommended,
            Layer::ConfigFile,
            Layer::Environment,
            Layer::Flags,
            Layer::BuilderOverride,
        ] {
            assert!(
                !Provenance::new(layer).locked,
                "{layer:?} must not be locked"
            );
        }
        assert!(Provenance::new(Layer::Enforced).locked);
    }

    #[test]
    fn layer_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(Layer::BuilderOverride).unwrap(),
            serde_json::json!("builder_override")
        );
    }
}
