//! [`ConfigFormat`]: the on-disk document shape a [`super::ConfigStore`] uses.

use super::ConfigError;

/// The serialization format [`super::ConfigStore`] reads and writes.
///
/// This is a property of the **store**, not the backend — see spec 016
/// "Format": backends deal in raw bytes only, so the same [`super::FileBackend`]
/// can hold a JSON document for one app and a TOML document for another.
///
/// JSON is the default: managed configuration (a later PRD) moves the same
/// document between a local file, an HTTP contract, and a renderer, and
/// keeping one format across all of those removes a class of conversion
/// bugs. TOML remains selectable for the CLI-tool case where a human hand-
/// edits the file and wants comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigFormat {
    #[default]
    Json,
    Toml,
}

impl ConfigFormat {
    /// Parse raw bytes into a generic [`serde_json::Value`], regardless of
    /// which format they were encoded in. Used as the common intermediate
    /// representation the migration pipeline operates on.
    pub(crate) fn bytes_to_value(
        self,
        backend_label: &str,
        bytes: &[u8],
    ) -> Result<serde_json::Value, ConfigError> {
        match self {
            ConfigFormat::Json => serde_json::from_slice(bytes).map_err(|e| ConfigError::Parse {
                backend: backend_label.to_string(),
                source: Box::new(e),
            }),
            ConfigFormat::Toml => {
                let text = std::str::from_utf8(bytes).map_err(|e| ConfigError::Parse {
                    backend: backend_label.to_string(),
                    source: Box::new(e),
                })?;
                let toml_value: toml::Value =
                    toml::from_str(text).map_err(|e| ConfigError::Parse {
                        backend: backend_label.to_string(),
                        source: Box::new(e),
                    })?;
                serde_json::to_value(toml_value).map_err(|e| ConfigError::Parse {
                    backend: backend_label.to_string(),
                    source: Box::new(e),
                })
            }
        }
    }

    /// Serialize a generic [`serde_json::Value`] into raw bytes in this
    /// format.
    pub(crate) fn value_to_bytes(
        self,
        backend_label: &str,
        value: &serde_json::Value,
    ) -> Result<Vec<u8>, ConfigError> {
        match self {
            ConfigFormat::Json => {
                serde_json::to_vec_pretty(value).map_err(|e| ConfigError::Serialize {
                    backend: backend_label.to_string(),
                    source: Box::new(e),
                })
            }
            ConfigFormat::Toml => {
                let toml_value: toml::Value =
                    serde_json::from_value(value.clone()).map_err(|e| ConfigError::Serialize {
                        backend: backend_label.to_string(),
                        source: Box::new(e),
                    })?;
                toml::to_string_pretty(&toml_value)
                    .map(|s| s.into_bytes())
                    .map_err(|e| ConfigError::Serialize {
                        backend: backend_label.to_string(),
                        source: Box::new(e),
                    })
            }
        }
    }
}

// `bytes_to_value` / `value_to_bytes` are `pub(crate)`, reachable only from
// within this crate — hence an inline test module here (the house exception
// for tiny pure-function helpers) rather than an external `tests/` file,
// which could not call them directly at all.
//
// Two of the four TOML error arms are exercised through `ConfigStore` in
// `tests/unit/config_store.rs` (invalid UTF-8, invalid syntax, a `null`
// field that TOML cannot represent). The remaining two are effectively
// unreachable through any value `ConfigStore` ever actually constructs:
// `serde_json::to_value` on an already-valid `toml::Value` and
// `serde_json::to_vec_pretty` on an already-valid `serde_json::Value` do not
// fail for any value produced by this crate's own conversions (JSON's `Value`
// cannot even represent a non-finite float — `Number::from_f64` returns
// `None` for one — so nothing reachable here trips the JSON serializer).
// What *is* directly testable, and only from inside the crate: TOML's
// requirement that the document root be a table, not a bare scalar or array —
// triggered here by calling `value_to_bytes` with inputs `ConfigStore` itself
// would never pass (it only ever passes an object, since `T` is always a
// struct), but which the function's own signature does not rule out.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_root_must_be_a_table_not_a_bare_string() {
        let err = ConfigFormat::Toml
            .value_to_bytes(
                "test-backend",
                &serde_json::Value::String("bare".to_string()),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::Serialize { .. }));
    }

    #[test]
    fn toml_root_must_be_a_table_not_a_bare_array() {
        let err = ConfigFormat::Toml
            .value_to_bytes(
                "test-backend",
                &serde_json::Value::Array(vec![serde_json::Value::from(1)]),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::Serialize { .. }));
    }
}
