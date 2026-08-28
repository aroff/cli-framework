//! `RegistryBackend` round-trip behavior.
//!
//! Windows-only (spec 016 user story 7): the whole file compiles out to an
//! empty, trivially-passing test binary everywhere else, rather than a Linux
//! stand-in pretending to exercise the registry. Registered unconditionally
//! in `Cargo.toml` (`required-features = ["config"]`) so CI on Windows
//! actually exercises it; on Linux/macOS `cargo test` reports `0 passed` for
//! this binary, which is the intended, honest outcome.
#![cfg(windows)]

use cli_framework::config::{ConfigBackend, RegistryBackend};

/// A unique-per-run subkey path so parallel/repeated test runs on the same
/// machine don't collide with each other's leftover registry state.
fn unique_subkey(name: &str) -> String {
    format!(
        "Software\\cli-framework-config-tests\\{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn cleanup(subkey: &str) {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // Best-effort: remove the leaf key created by the test.
    let _ = hkcu.delete_subkey_all(subkey);
}

// User story 6 — full round trip under a test key.
#[test]
fn registry_backend_round_trips() {
    let subkey = unique_subkey("roundtrip");
    let backend = RegistryBackend::new(subkey.clone(), "config");

    backend.write(b"{\"greeting\":\"hi\"}").unwrap();
    assert_eq!(backend.read().unwrap(), b"{\"greeting\":\"hi\"}".to_vec());

    // Overwrite works too (not just first-write).
    backend.write(b"{\"greeting\":\"bye\"}").unwrap();
    assert_eq!(backend.read().unwrap(), b"{\"greeting\":\"bye\"}".to_vec());

    cleanup(&subkey);
}

// Absent key reads as defaults (empty bytes), matching FileBackend's
// "nothing stored yet" contract.
#[test]
fn absent_key_reads_as_empty() {
    let subkey = unique_subkey("absent");
    let backend = RegistryBackend::new(subkey, "config");
    assert_eq!(backend.read().unwrap(), Vec::<u8>::new());
}

#[test]
fn for_app_uses_software_key_under_hkcu() {
    let backend = RegistryBackend::for_app("my-test-app");
    assert!(backend.label().contains("my-test-app"));
    assert!(backend.label().starts_with("registry:HKCU\\"));
}

#[test]
fn supports_write_is_always_true() {
    let backend = RegistryBackend::new(unique_subkey("supports-write"), "config");
    assert!(backend.supports_write());
}
