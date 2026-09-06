// src/telemetry/store.rs
//! The framework's own settings file,
//! `<config_dir>/<app>/telemetry.<json|toml>`.
//!
//! Telemetry state is deliberately **not** kept in the application's config
//! backend: consent has to be readable and writable identically everywhere,
//! including where the app's own backend is the Windows registry. It is
//! always a file, written through the crate's [`ConfigStore`] so it gets
//! atomic write-and-rename and schema versioning for free.
//!
//! The *format* still follows the application: an app that declares TOML gets
//! a `telemetry.toml` next to its own configuration, not a lone JSON file a
//! user hand-editing that directory would not expect. JSON is the default,
//! used when an app declares no configuration of its own.
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

// Hand-written, not `#[derive(Default)]`: the variant that should be default
// (`Unavailable`) carries a `String`, and `#[default]` only accepts a unit
// variant. A default that claimed the store was `Ready` would make an
// unconfigured `StartupReport` (PR4 Task 16) lie in exactly the direction
// that hides a bug, so the fallback reason is spelled out here instead.
impl Default for StoreState {
    fn default() -> Self {
        StoreState::Unavailable("not opened".into())
    }
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

/// The settings file's name for a given configuration format. Exhaustive on
/// purpose: a new [`ConfigFormat`] variant has to choose its extension here
/// rather than silently inheriting `.json`.
fn settings_file_name(format: ConfigFormat) -> &'static str {
    match format {
        ConfigFormat::Json => "telemetry.json",
        ConfigFormat::Toml => "telemetry.toml",
    }
}

/// Reads and writes [`TelemetrySettings`].
pub struct TelemetryStore {
    state: StoreState,
    store: Option<ConfigStore<TelemetrySettings>>,
}

impl TelemetryStore {
    /// Open under an explicit parent directory, in JSON — the format an app
    /// that declares no configuration of its own gets. Tests use this;
    /// production goes through [`Self::open`] or [`Self::open_with_format`].
    pub fn open_at(config_dir: impl AsRef<Path>, app: &str) -> Self {
        Self::open_at_with_format(config_dir, app, ConfigFormat::Json)
    }

    /// Open under an explicit parent directory, in `format`. The file name
    /// follows the format, so a TOML app's telemetry settings sit beside its
    /// own TOML configuration rather than in a lone JSON file.
    pub fn open_at_with_format(
        config_dir: impl AsRef<Path>,
        app: &str,
        format: ConfigFormat,
    ) -> Self {
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
        let path = app_dir.join(settings_file_name(format));
        let backend = Arc::new(FileBackend::new(path.clone()));
        Self {
            state: StoreState::Ready(path),
            store: Some(ConfigStore::new(backend, format, TELEMETRY_SCHEMA_VERSION)),
        }
    }

    /// Open under the platform config directory, in JSON.
    pub fn open(app: &str) -> Self {
        Self::open_with_format(app, ConfigFormat::Json)
    }

    /// Open under the platform config directory, in `format`. This is what
    /// startup calls, passing the format the application declared.
    pub fn open_with_format(app: &str, format: ConfigFormat) -> Self {
        match dirs::config_dir() {
            Some(dir) => Self::open_at_with_format(dir, app, format),
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

    /// Delete the settings file, so the next run starts over as a new
    /// Install: new id, no stored consent, notice shown again.
    ///
    /// ADR 0077 makes this a privacy affordance, not a "clear my preferences"
    /// convenience — `telemetry reset` *means* "fresh install", and the ADR
    /// leans on that meaning to justify keeping the whole `telemetry` subtree
    /// out of roaming ("it cannot mean that if the next sync brings the old
    /// level back"). Carrying the id across a reset would defeat it: a person
    /// who asked to be forgotten would stay joinable to everything the old id
    /// had already sent.
    ///
    /// Removing the file rather than writing defaults over it is the same
    /// decision: ADR 0077 has "level, id, notice marker and probe switches go
    /// together", and a file left behind is residue on a disk the person just
    /// asked to clear. No reader needs changing — an absent file already
    /// reads as defaults (see [`Self::settings`]).
    ///
    /// Resetting an Install that stored nothing succeeds rather than erroring:
    /// the post-condition it promises already holds.
    pub fn reset(&self) -> Result<(), ConfigError> {
        let StoreState::Ready(path) = &self.state else {
            return Err(ConfigError::ReadOnly {
                backend: self.state.describe(),
            });
        };
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(ConfigError::BackendWrite {
                backend: path.display().to_string(),
                source: Box::new(err),
            }),
        }
    }

    /// A store that was never opened against a real path — reads fall back to
    /// defaults and every write fails with `reason`.
    ///
    /// Test-only constructor: production always goes through [`Self::open`],
    /// [`Self::open_with_format`], [`Self::open_at`] or
    /// [`Self::open_at_with_format`], which compute their own reason when the
    /// platform or directory is unavailable.
    #[doc(hidden)]
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            state: StoreState::Unavailable(reason.into()),
            store: None,
        }
    }
}

/// Where a process's telemetry settings file lives.
///
/// `dir: None` means the platform configuration directory, which is what
/// every real application uses. A test sets it to a `TempDir` so it never
/// touches the person's own consent file. `format` is JSON until PR7's
/// Task 27 teaches the builder to report the application's declared
/// configuration format (PRD line 258) — do not try to resolve it here.
#[derive(Debug, Clone, Default)]
pub struct TelemetryStoreLocation {
    pub dir: Option<PathBuf>,
    pub format: ConfigFormat,
}

impl TelemetryStoreLocation {
    pub fn open(&self, app: &str) -> TelemetryStore {
        match &self.dir {
            Some(dir) => TelemetryStore::open_at_with_format(dir, app, self.format),
            None => TelemetryStore::open_with_format(app, self.format),
        }
    }
}
