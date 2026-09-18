//! The install receipt: what `self install` did, so `self uninstall` can
//! reverse exactly that and nothing else.

use super::method::InstallMethod;
use super::options::{ReleaseSource, SelfInstallOptions};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const RECEIPT_SCHEMA_VERSION: u32 = 1;

/// Where the application's releases come from, as recorded at install time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptSource {
    #[serde(flatten)]
    pub source: ReleaseSource,
    pub tag_prefix: String,
    pub asset_template: String,
}

impl From<&SelfInstallOptions> for ReceiptSource {
    fn from(o: &SelfInstallOptions) -> Self {
        Self {
            source: o.source.clone(),
            tag_prefix: o.tag_prefix.clone(),
            asset_template: o.asset_template.clone(),
        }
    }
}

/// One change made outside the bin dir to put it on PATH.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PathModification {
    /// The shared `env` or `env.fish` file was created. Shared with sibling
    /// apps, so uninstall keeps it.
    EnvFile { file: PathBuf },
    /// One line appended to an existing rc file. Shared, so uninstall keeps
    /// it.
    RcLine { file: PathBuf, line: String },
    /// A per-app fish `conf.d` file. Removed on uninstall.
    FishConf { file: PathBuf },
    /// The bin dir was prepended to the user `Path` in the registry.
    /// Removed on uninstall only when no other executable remains in it.
    WindowsUserPath { key: String, entry: String },
}

/// Schema version 1 of `install-receipt.json`. Phase 2 and 3 fields are
/// optional and additive, so a phase 1 binary still reads the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallReceipt {
    pub schema_version: u32,
    pub app: String,
    pub version: String,
    pub target: String,
    pub channel: String,
    pub bin_dir: PathBuf,
    pub binary_path: PathBuf,
    pub method: InstallMethod,
    #[serde(default)]
    pub modified_path: Vec<PathModification>,
    #[serde(default)]
    pub completions: Vec<PathBuf>,
    pub source: ReceiptSource,
    /// RFC 3339, UTC: when the binary at `binary_path` was last placed.
    pub installed_at: String,
    /// The version kept beside the binary as `<binary>.prev` by the last
    /// `self update`, which `self rollback` restores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    /// Installed with `--system` into a machine-wide bin dir.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub system: bool,
    /// The `HKCU` Apps & Features key this install created (Windows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps_and_features: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReceiptError {
    #[error("cannot read install receipt {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("install receipt {path} is not valid: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("cannot write install receipt {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl InstallReceipt {
    /// `Ok(None)` when no receipt exists.
    pub fn load(path: &Path) -> Result<Option<Self>, ReceiptError> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ReceiptError::Read {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        let receipt: Self = serde_json::from_slice(&bytes).map_err(|e| ReceiptError::Invalid {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        if receipt.schema_version != RECEIPT_SCHEMA_VERSION {
            return Err(ReceiptError::Invalid {
                path: path.to_path_buf(),
                reason: format!(
                    "schema version {} is not supported (expected {RECEIPT_SCHEMA_VERSION})",
                    receipt.schema_version
                ),
            });
        }
        Ok(Some(receipt))
    }

    /// Write atomically: a temporary file in the same directory, then a
    /// rename, so a crash never leaves half a receipt.
    pub fn save(&self, path: &Path) -> Result<(), ReceiptError> {
        let write_err = |source| ReceiptError::Write {
            path: path.to_path_buf(),
            source,
        };
        let dir = path.parent().ok_or_else(|| {
            write_err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "receipt path has no parent directory",
            ))
        })?;
        std::fs::create_dir_all(dir).map_err(write_err)?;
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| write_err(std::io::Error::other(e.to_string())))?;
        let tmp = dir.join(format!(".install-receipt.{}.tmp", std::process::id()));
        std::fs::write(&tmp, json).map_err(write_err)?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            write_err(e)
        })
    }
}

/// Now as RFC 3339 in UTC, second precision, without a date dependency.
pub(crate) fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    rfc3339_from_unix(secs)
}

pub(crate) fn rfc3339_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1_789_561_845), "2026-09-16T12:30:45Z");
    }
}
