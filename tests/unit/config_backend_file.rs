//! `FileBackend` behavior: empty reads, round trips, missing-parent-dir
//! auto-creation, atomicity, and labeling. Mirrors
//! `tests/unit/secrets_conformance.rs`'s style of testing a concrete backend
//! against a real (temporary) filesystem.

use cli_framework::config::{ConfigBackend, ConfigError, FileBackend};
use tempfile::TempDir;

// User story 5 (backend half) — an absent file reads as empty bytes, not an
// error. `ConfigStore` is what maps that to defaults; the backend's own
// contract is just "no error, no bytes".
#[test]
fn read_missing_file_returns_empty_bytes() {
    let dir = TempDir::new().unwrap();
    let backend = FileBackend::new(dir.path().join("does-not-exist.json"));
    assert_eq!(backend.read().unwrap(), Vec::<u8>::new());
}

// Round trip: write then read returns exactly what was written.
#[test]
fn write_then_read_round_trips() {
    let dir = TempDir::new().unwrap();
    let backend = FileBackend::new(dir.path().join("cfg.json"));
    backend.write(b"{\"greeting\":\"hi\"}").unwrap();
    assert_eq!(backend.read().unwrap(), b"{\"greeting\":\"hi\"}".to_vec());
}

// User story 4 — first save creates missing parent directories.
#[test]
fn write_creates_missing_parent_directories() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("a").join("b").join("c").join("cfg.json");
    assert!(!nested.parent().unwrap().exists());

    let backend = FileBackend::new(&nested);
    backend.write(b"hello").unwrap();

    assert!(nested.exists());
    assert_eq!(std::fs::read(&nested).unwrap(), b"hello");
}

// After a successful write, the temporary file used for the atomic
// tmp-then-rename does not survive — only the target remains.
#[test]
fn successful_write_leaves_no_temp_file_behind() {
    let dir = TempDir::new().unwrap();
    let backend = FileBackend::new(dir.path().join("cfg.json"));
    backend.write(b"v1").unwrap();

    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["cfg.json".to_string()]);
}

// User story 3 — atomicity: a write that fails partway (simulated by
// removing the directory's write permission, so the temp file can't even be
// created) leaves the previous file readable and unchanged, and no temp file
// is left behind. This is a genuine negative check: a naive non-atomic
// implementation that opens the *existing* target file directly (rather than
// tmp-then-rename) would still succeed here, because modifying an existing
// file's contents needs only the file's own write permission, not the
// directory's — so this test fails if atomicity regresses to direct
// overwrite.
#[cfg(unix)]
#[test]
fn failed_write_leaves_previous_file_intact_and_no_temp_leak() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.json");
    let backend = FileBackend::new(&path);
    backend.write(b"first").unwrap();

    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    perms.set_mode(0o500); // r-x: existing files can be read, nothing new created
    std::fs::set_permissions(dir.path(), perms).unwrap();

    let result = backend.write(b"second");

    // Restore permissions before any assertion can fail this test and skip
    // cleanup of the TempDir.
    let mut restored = std::fs::metadata(dir.path()).unwrap().permissions();
    restored.set_mode(0o700);
    std::fs::set_permissions(dir.path(), restored).unwrap();

    assert!(result.is_err(), "write into a read-only dir must fail");
    assert!(matches!(result.unwrap_err(), ConfigError::Io { .. }));

    assert_eq!(std::fs::read(&path).unwrap(), b"first");

    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["cfg.json".to_string()],
        "no leftover temp file after a failed write"
    );
}

// `label()` names the file path, for `doctor` / diagnostics (user story 22).
#[test]
fn label_contains_the_path() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.json");
    let backend = FileBackend::new(&path);
    assert!(backend.label().contains(&path.display().to_string()));
}

#[test]
fn path_accessor_returns_configured_path() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.json");
    let backend = FileBackend::new(&path);
    assert_eq!(backend.path(), path);
}

#[test]
fn supports_write_is_always_true() {
    let dir = TempDir::new().unwrap();
    let backend = FileBackend::new(dir.path().join("cfg.json"));
    assert!(backend.supports_write());
}

// `for_app` resolves under the platform config directory (redirected here via
// `$HOME`/`$XDG_CONFIG_HOME` so the test never touches the real user profile)
// and names the file after the app, without hardcoding a format extension.
//
// Note: `for_app`'s `ConfigError::Io` branch (no resolvable config directory
// at all) is not exercised here. `dirs::config_dir()` on Linux falls back to
// a `getpwuid`-style OS user-database lookup when `$HOME`/`$XDG_CONFIG_HOME`
// are both unset, so a real user account (as opposed to a stripped-down
// container with no user database entry at all) can't be made to hit that
// branch portably from a test — confirmed by trying exactly that and finding
// `dirs::config_dir()` still resolves.
#[test]
fn for_app_resolves_under_redirected_config_dir() {
    let dir = TempDir::new().unwrap();
    let original_xdg = std::env::var("XDG_CONFIG_HOME").ok();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());

    let backend = FileBackend::for_app("my-test-app").unwrap();

    match original_xdg {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }

    assert!(backend.path().starts_with(dir.path()));
    assert!(backend.path().to_string_lossy().contains("my-test-app"));

    backend.write(b"{}").unwrap();
    assert_eq!(backend.read().unwrap(), b"{}".to_vec());
}

// `write` short-circuits with `ConfigError::Io` when the target path has no
// parent component at all (e.g. the filesystem root) — never touching the
// filesystem, since there is nothing sensible to create or rename.
#[test]
fn write_to_a_path_with_no_parent_returns_io_error() {
    let backend = FileBackend::new("/");
    let err = backend.write(b"x").unwrap_err();
    assert!(matches!(err, ConfigError::Io { .. }));
}

// `write` maps a `create_dir_all` failure (an ancestor component exists but
// is a regular file, not a directory) to `ConfigError::Io`.
#[test]
fn write_when_ancestor_is_a_file_returns_io_error() {
    let dir = TempDir::new().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"i am a file, not a directory").unwrap();

    let backend = FileBackend::new(blocker.join("nested").join("cfg.json"));
    let err = backend.write(b"x").unwrap_err();
    assert!(matches!(err, ConfigError::Io { .. }));
}

// `write`'s final rename fails (and is surfaced, not swallowed) when the
// target path already exists as a directory: the temp file is created and
// written successfully, but `rename` cannot replace a directory with a file.
#[test]
fn write_when_target_is_a_directory_returns_io_error_from_rename() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("cfg.json");
    std::fs::create_dir(&target).unwrap();

    let backend = FileBackend::new(&target);
    let err = backend.write(b"x").unwrap_err();
    assert!(matches!(err, ConfigError::Io { .. }));
}

// `read` maps a non-`NotFound` I/O failure (permission denied) to
// `ConfigError::Io`, distinct from the "absent file -> empty bytes" contract.
#[cfg(unix)]
#[test]
fn read_permission_denied_returns_io_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cfg.json");
    std::fs::write(&path, b"secret").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

    let backend = FileBackend::new(&path);
    let result = backend.read();

    // Restore permissions unconditionally so TempDir cleanup can proceed.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    assert!(matches!(result, Err(ConfigError::Io { .. })));
}

// Regression: two `FileBackend` instances over the *same path*, written
// concurrently from different threads in the *same process*, must never
// interleave into a corrupted file. The temp file name used to be
// `{file}.tmp.{pid}` — identical for every writer in one process — so two
// concurrent writers could race on `File::create` (which truncates) and each
// other's `write_all`/rename, producing a file that is neither writer's
// intended content. Each writer here writes a distinguishable, uniform
// payload (all-`A` vs all-`B`) many times; the fix (a per-write counter mixed
// into the temp name) means every write gets its own temp file no matter how
// many instances or threads target the same path, so the file on disk must
// always be fully one payload or the other, never a byte-level mix.
#[test]
fn concurrent_writes_from_two_instances_over_the_same_path_never_interleave() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("shared.json");
    // Seed the file so both backends' first write has something to replace.
    std::fs::write(&path, vec![b'A'; 4096]).unwrap();

    let path_a = path.clone();
    let path_b = path.clone();
    let writer_a = std::thread::spawn(move || {
        let backend = FileBackend::new(&path_a);
        for _ in 0..200 {
            backend.write(&vec![b'A'; 4096]).unwrap();
        }
    });
    let writer_b = std::thread::spawn(move || {
        let backend = FileBackend::new(&path_b);
        for _ in 0..200 {
            backend.write(&vec![b'B'; 4096]).unwrap();
        }
    });
    writer_a.join().unwrap();
    writer_b.join().unwrap();

    let final_bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        final_bytes.len(),
        4096,
        "file must be a complete write, not a truncated fragment"
    );
    assert!(
        final_bytes.iter().all(|&b| b == b'A') || final_bytes.iter().all(|&b| b == b'B'),
        "final file mixes both writers' bytes — a temp-file collision corrupted a write"
    );

    // No leaked temp files: every writer either renamed its own uniquely
    // named temp file away, or cleaned it up on error (none should have
    // errored here).
    let leftover: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp."))
        .collect();
    assert!(leftover.is_empty(), "leaked temp files: {leftover:?}");
}
