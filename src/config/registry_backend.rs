//! [`RegistryBackend`]: a [`ConfigBackend`] backed by the Windows registry.
//!
//! Compiled only on Windows (`#[cfg(windows)]`) — the same source tree builds
//! on Linux and macOS with this type simply absent (spec 016 user story 7),
//! so a cross-platform application never needs conditional code of its own to
//! avoid referencing it.

use super::{ConfigBackend, ConfigError};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

/// Stores the serialized document as a single `REG_SZ` string value under an
/// application-owned key in `HKEY_CURRENT_USER`.
///
/// An absent key or absent value reads as empty bytes (spec 016 user story
/// 5's "empty or absent backend yields defaults" applies here exactly as it
/// does for [`super::FileBackend`]).
pub struct RegistryBackend {
    subkey_path: String,
    value_name: String,
}

impl RegistryBackend {
    /// Store the document under `HKCU\Software\<app_name>\Config`, value
    /// name `config`.
    pub fn for_app(app_name: &str) -> Self {
        Self::new(format!("Software\\{app_name}\\Config"), "config")
    }

    /// Store the document under an explicit subkey path (relative to
    /// `HKEY_CURRENT_USER`) and value name.
    pub fn new(subkey_path: impl Into<String>, value_name: impl Into<String>) -> Self {
        Self {
            subkey_path: subkey_path.into(),
            value_name: value_name.into(),
        }
    }
}

impl ConfigBackend for RegistryBackend {
    fn read(&self) -> Result<Vec<u8>, ConfigError> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = match hkcu.open_subkey_with_flags(&self.subkey_path, KEY_READ) {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(ConfigError::backend_read(self.label(), e)),
        };
        match key.get_value::<String, _>(&self.value_name) {
            Ok(s) => Ok(s.into_bytes()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(ConfigError::backend_read(self.label(), e)),
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), ConfigError> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _disposition) = hkcu
            .create_subkey_with_flags(&self.subkey_path, KEY_WRITE)
            .map_err(|e| ConfigError::backend_write(self.label(), e))?;
        let text = String::from_utf8(bytes.to_vec())
            .map_err(|e| ConfigError::backend_write(self.label(), e))?;
        key.set_value(&self.value_name, &text)
            .map_err(|e| ConfigError::backend_write(self.label(), e))?;
        Ok(())
    }

    fn supports_write(&self) -> bool {
        true
    }

    fn label(&self) -> String {
        format!("registry:HKCU\\{}\\{}", self.subkey_path, self.value_name)
    }
}
