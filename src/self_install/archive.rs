//! Verifying and unpacking a release archive.
//!
//! The release contract (ADR 0080): each archive is listed in `SHA256SUMS`
//! in `sha256sum` format with bare file names, and holds the binary at its
//! top level. When the application declares a minisign public key,
//! `SHA256SUMS.minisig` signs the sums file, so one signature covers every
//! asset of the release.
//!
//! Only the binary is extracted, to a path chosen here, so no archive entry
//! name ever becomes a write path. An archive that names any entry with an
//! absolute path or a `..` component is still rejected outright: it was not
//! produced by the release workflow, and nothing else in it can be trusted.

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

/// The checksum file every release publishes.
pub const SUMS_FILE: &str = "SHA256SUMS";
/// The minisign signature of [`SUMS_FILE`].
pub const SIGNATURE_FILE: &str = "SHA256SUMS.minisig";

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{asset} is not listed in {SUMS_FILE}")]
    NotListed { asset: String },
    #[error("checksum mismatch for {asset}: expected {expected}, got {actual}")]
    Mismatch {
        asset: String,
        expected: String,
        actual: String,
    },
    #[error("{SIGNATURE_FILE} does not verify against the application's public key: {0}")]
    BadSignature(String),
    #[error("{0} has no {SIGNATURE_FILE}, and this application only accepts signed releases")]
    Unsigned(String),
    #[error("the application's minisign public key is invalid: {0}")]
    BadPublicKey(String),
    #[error("{archive} holds an unsafe entry path {entry:?}")]
    UnsafeEntry { archive: PathBuf, entry: String },
    #[error("{archive} does not contain {binary}")]
    MissingBinary { archive: PathBuf, binary: String },
    #[error("{0} is neither a .tar.gz nor a .zip archive")]
    UnknownFormat(PathBuf),
    #[error("{archive} is not a valid archive: {message}")]
    Corrupt { archive: PathBuf, message: String },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ArchiveError + '_ {
    move |source| ArchiveError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Lowercase hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> Result<String, ArchiveError> {
    let mut file = File::open(path).map_err(io_err(path))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(io_err(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// The digest `sums` records for `asset`. Accepts the text (`hash  name`)
/// and binary (`hash *name`) forms of `sha256sum`, as the installer scripts
/// do.
pub fn expected_digest(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim_end().split_once(char::is_whitespace)?;
        let name = name.trim_start();
        let name = name.strip_prefix('*').unwrap_or(name);
        (name == asset && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// Check `archive` against the digest `sums` records for `asset`.
pub fn verify_checksum(archive: &Path, asset: &str, sums: &str) -> Result<(), ArchiveError> {
    let expected = expected_digest(sums, asset).ok_or_else(|| ArchiveError::NotListed {
        asset: asset.to_string(),
    })?;
    let actual = sha256_file(archive)?;
    if actual != expected {
        return Err(ArchiveError::Mismatch {
            asset: asset.to_string(),
            expected,
            actual,
        });
    }
    Ok(())
}

/// Verify a minisign signature over the bytes of `SHA256SUMS`.
///
/// `public_key` is the base64 key line of a minisign `.pub` file (the
/// `RW...` string), or the whole file including its comment line.
pub fn verify_signature(
    sums: &[u8],
    signature: &str,
    public_key: &str,
) -> Result<(), ArchiveError> {
    let key = public_key.trim();
    let key = if key.contains('\n') {
        minisign_verify::PublicKey::decode(key)
    } else {
        minisign_verify::PublicKey::from_base64(key)
    }
    .map_err(|e| ArchiveError::BadPublicKey(e.to_string()))?;
    let signature = minisign_verify::Signature::decode(signature)
        .map_err(|e| ArchiveError::BadSignature(e.to_string()))?;
    // Legacy (non-prehashed) signatures are refused: minisign has produced
    // prehashed ones by default since 0.8.
    key.verify(sums, &signature, false)
        .map_err(|e| ArchiveError::BadSignature(e.to_string()))
}

/// Validate the public key at build time, so a typo fails the app's own
/// tests rather than a user's update.
pub fn check_public_key(public_key: &str) -> Result<(), String> {
    let key = public_key.trim();
    let parsed = if key.contains('\n') {
        minisign_verify::PublicKey::decode(key)
    } else {
        minisign_verify::PublicKey::from_base64(key)
    };
    parsed
        .map(|_| ())
        .map_err(|e| format!("self-install public key is not a minisign public key: {e}"))
}

/// Whether an entry name is safe: relative, with no `..`, root or drive
/// prefix. Backslashes count as separators, as a Windows extractor would
/// treat them.
pub fn is_safe_entry(name: &str) -> bool {
    if name.is_empty() || name.starts_with('/') || name.starts_with('\\') || name.contains(':') {
        return false;
    }
    let normalized = name.replace('\\', "/");
    Path::new(&normalized)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Whether the entry is the binary: at the top level, or one directory down
/// (the layout some older release workflows produced).
fn names_binary(name: &str, binary: &str) -> bool {
    let normalized = name.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    match parts.as_slice() {
        [file] => *file == binary,
        [_dir, file] => *file == binary,
        _ => false,
    }
}

/// Extract `binary` from `archive` into `dest_dir`, returning its path.
///
/// Every entry name is checked before anything is written; one unsafe name
/// rejects the archive.
pub fn extract_binary(
    archive: &Path,
    binary: &str,
    dest_dir: &Path,
) -> Result<PathBuf, ArchiveError> {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let out = dest_dir.join(binary);
    let found = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(archive, binary, &out)?
    } else if name.ends_with(".zip") {
        extract_zip(archive, binary, &out)?
    } else {
        return Err(ArchiveError::UnknownFormat(archive.to_path_buf()));
    };
    if !found {
        let _ = std::fs::remove_file(&out);
        return Err(ArchiveError::MissingBinary {
            archive: archive.to_path_buf(),
            binary: binary.to_string(),
        });
    }
    make_executable(&out).map_err(io_err(&out))?;
    Ok(out)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn corrupt(archive: &Path, e: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::Corrupt {
        archive: archive.to_path_buf(),
        message: e.to_string(),
    }
}

fn unsafe_entry(archive: &Path, entry: &str) -> ArchiveError {
    ArchiveError::UnsafeEntry {
        archive: archive.to_path_buf(),
        entry: entry.to_string(),
    }
}

fn extract_tar_gz(archive: &Path, binary: &str, out: &Path) -> Result<bool, ArchiveError> {
    // Pass 1: every name must be safe. Pass 2: copy the binary. Two passes
    // over a gzip stream cost a second decompression of a few megabytes and
    // guarantee nothing is written from an archive that is later rejected.
    let open = || -> Result<tar::Archive<flate2::read::GzDecoder<File>>, ArchiveError> {
        let file = File::open(archive).map_err(io_err(archive))?;
        Ok(tar::Archive::new(flate2::read::GzDecoder::new(file)))
    };
    let mut tar = open()?;
    for entry in tar.entries().map_err(|e| corrupt(archive, e))? {
        let entry = entry.map_err(|e| corrupt(archive, e))?;
        let path = entry.path_bytes();
        let name = String::from_utf8_lossy(&path).into_owned();
        if !is_safe_entry(&name) {
            return Err(unsafe_entry(archive, &name));
        }
    }
    let mut tar = open()?;
    for entry in tar.entries().map_err(|e| corrupt(archive, e))? {
        let mut entry = entry.map_err(|e| corrupt(archive, e))?;
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        if names_binary(&name, binary) && entry.header().entry_type().is_file() {
            let mut file = File::create(out).map_err(io_err(out))?;
            io::copy(&mut entry, &mut file).map_err(|e| corrupt(archive, e))?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn extract_zip(archive: &Path, binary: &str, out: &Path) -> Result<bool, ArchiveError> {
    let file = File::open(archive).map_err(io_err(archive))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| corrupt(archive, e))?;
    let mut wanted = None;
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).map_err(|e| corrupt(archive, e))?;
        let name = entry.name().to_string();
        if !is_safe_entry(&name) || entry.enclosed_name().is_none() {
            return Err(unsafe_entry(archive, &name));
        }
        if wanted.is_none() && entry.is_file() && names_binary(&name, binary) {
            wanted = Some(i);
        }
    }
    let Some(index) = wanted else {
        return Ok(false);
    };
    let mut entry = zip.by_index(index).map_err(|e| corrupt(archive, e))?;
    let mut file = File::create(out).map_err(io_err(out))?;
    io::copy(&mut entry, &mut file).map_err(|e| corrupt(archive, e))?;
    Ok(true)
}

/// Ask a binary for its version: `<binary> --version`, first token that
/// parses as semver (a leading `v` is allowed). This is also a smoke test:
/// a binary that cannot run on this machine fails here, before it replaces
/// the working one.
pub fn probe_version(binary: &Path) -> Result<semver::Version, String> {
    let mut attempt = 0;
    let output = loop {
        match std::process::Command::new(binary).arg("--version").output() {
            Ok(output) => break output,
            // ETXTBSY: another thread of this process forked while the file
            // was still open for writing. Short-lived; wait it out.
            Err(e) if e.raw_os_error() == Some(26) && cfg!(unix) && attempt < 10 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(format!("cannot run {}: {e}", binary.display())),
        }
    };
    if !output.status.success() {
        return Err(format!(
            "{} --version exited with {}",
            binary.display(),
            output.status
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .find_map(|t| semver::Version::parse(t.trim_start_matches('v')).ok())
        .ok_or_else(|| {
            format!(
                "{} --version printed no version: {:?}",
                binary.display(),
                text.trim()
            )
        })
}
