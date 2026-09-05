// src/telemetry/store.rs
//! The framework's own settings file, `<config_dir>/<app>/telemetry.json`.
//!
//! Telemetry state is deliberately **not** kept in the application's config
//! backend: consent has to be readable and writable identically everywhere,
//! including where the app's own backend is the Windows registry. It is
//! always a file, written through the crate's [`ConfigStore`] so it gets
//! atomic write-and-rename and schema versioning for free.
//!
//! A store whose directory cannot be created is [`StoreState::Unavailable`].
//! That is never a startup failure: reads fall back to defaults, Attribution
//! degrades to anonymous, writes fail with the reason, and the doctor reports
//! it.

use super::axes::{Attribution, TelemetryLevel};
use crate::config::{ConfigError, ConfigFormat, ConfigStore, FileBackend, VersionedConfig};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Schema version of [`TelemetrySettings`].
pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;

/// What an Install has stored about its own telemetry. Every field is
/// optional because "not chosen" and "chosen to be off" are different states:
/// the first can still be raised by an organisation recommendation, the
/// second cannot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetrySettings {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<TelemetryLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<Attribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_id: Option<String>,
    /// The telemetry level that was announced when the notice was last shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice_shown: Option<TelemetryLevel>,
    /// `telemetry.<probe>.enabled` overrides. Absent means enabled.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub probes: BTreeMap<String, bool>,
}

impl VersionedConfig for TelemetrySettings {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

/// Whether the settings file is usable, and why not when it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreState {
    Ready(PathBuf),
    Unavailable(String),
}

impl StoreState {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Ready(_) => None,
            Self::Unavailable(reason) => Some(reason),
        }
    }

    /// One line for `telemetry status` and the doctor.
    pub fn describe(&self) -> String {
        match self {
            Self::Ready(path) => path.display().to_string(),
            Self::Unavailable(reason) => format!("unavailable: {reason}"),
        }
    }
}

/// Reads and writes [`TelemetrySettings`].
pub struct TelemetryStore {
    state: StoreState,
    store: Option<ConfigStore<TelemetrySettings>>,
}

impl TelemetryStore {
    /// Open under an explicit parent directory. Tests use this; production
    /// goes through [`Self::open`].
    pub fn open_at(config_dir: impl AsRef<Path>, app: &str) -> Self {
        let app_dir = config_dir.as_ref().join(app);
        if let Err(err) = std::fs::create_dir_all(&app_dir) {
            return Self {
                state: StoreState::Unavailable(format!(
                    "cannot create {}: {err}",
                    app_dir.display()
                )),
                store: None,
            };
        }
        let path = app_dir.join("telemetry.json");
        let backend = Arc::new(FileBackend::new(path.clone()));
        Self {
            state: StoreState::Ready(path),
            store: Some(ConfigStore::new(
                backend,
                ConfigFormat::Json,
                TELEMETRY_SCHEMA_VERSION,
            )),
        }
    }

    /// Open under the platform config directory.
    pub fn open(app: &str) -> Self {
        match dirs::config_dir() {
            Some(dir) => Self::open_at(dir, app),
            None => Self {
                state: StoreState::Unavailable(
                    "this platform has no resolvable configuration directory".to_string(),
                ),
                store: None,
            },
        }
    }

    pub fn state(&self) -> &StoreState {
        &self.state
    }

    /// The stored settings, or defaults when the store is unavailable or the
    /// file is unreadable. Reading telemetry settings never fails a startup.
    pub fn settings(&self) -> TelemetrySettings {
        self.store
            .as_ref()
            .and_then(|s| s.load().ok())
            .unwrap_or_default()
    }

    /// Read, apply `f`, write back. Returns the store's reason when the store
    /// is unavailable, so `telemetry set` can print it.
    pub fn mutate(
        &self,
        f: impl FnOnce(&mut TelemetrySettings),
    ) -> Result<TelemetrySettings, ConfigError> {
        let Some(store) = self.store.as_ref() else {
            return Err(ConfigError::ReadOnly {
                backend: self.state.describe(),
            });
        };
        let mut settings = store.load().unwrap_or_default();
        f(&mut settings);
        store.save(&settings)?;
        Ok(settings)
    }

    /// Return the Install's id, minting one on first use. A concurrent first
    /// run may mint a second id; the writes are atomic, so the last writer
    /// wins and both processes then agree on the file's contents.
    pub fn ensure_install_id(&self) -> Option<String> {
        if let Some(existing) = self.settings().install_id {
            return Some(existing);
        }
        let minted = uuid::Uuid::new_v4().to_string();
        self.mutate(|s| {
            if s.install_id.is_none() {
                s.install_id = Some(minted.clone());
            }
        })
        .ok()
        .and_then(|s| s.install_id)
    }

    /// Forget every stored choice. The Install id is deliberately kept: reset
    /// returns the Install to "has not chosen", it does not fabricate a new
    /// Install.
    pub fn reset(&self) -> Result<(), ConfigError> {
        self.mutate(|s| {
            let install_id = s.install_id.take();
            *s = TelemetrySettings {
                install_id,
                ..Default::default()
            };
        })
        .map(|_| ())
    }
}
