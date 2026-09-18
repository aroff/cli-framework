// tests/unit/self_update.rs
//! `self update`, `self rollback`, `--from`, release archives, policy keys
//! and the update notice (ADR 0080, phases 2 and 3).
//!
//! Releases are served by a wiremock mirror on loopback (an HTTP source, the
//! same contract as a GitHub release). The "binaries" are shell scripts that
//! answer `--version`, so the end-to-end tests are Unix-only; archive,
//! signature and policy tests run everywhere.

// The helpers below serve the Unix-only end-to-end tests too.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use cli_framework::config::Policy;
use cli_framework::self_install::{
    check_public_key, current_target, expected_digest, extract_binary, install, is_safe_entry,
    notice_line, path_decision, pick_release, prev_path, probe_version, receipt_path, rollback,
    system_bin_dir, uninstall, update, verify_checksum, verify_signature, ArchiveError, Channel,
    GithubRelease, InstallEnv, InstallReceipt, InstallRequest, Os, PathDecision, SelfInstallError,
    SelfInstallOptions, SelfUpdatePolicy, UpdateAction, UpdateRequest, KEY_CHANNEL, KEY_ENABLED,
    KEY_MINIMUM_VERSION,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const APP: &str = "demoapp";

fn binary_name() -> String {
    if Os::current().is_windows() {
        format!("{APP}.exe")
    } else {
        APP.to_string()
    }
}

/// A "binary" that reports `version`.
fn script(version: &str) -> Vec<u8> {
    format!("#!/bin/sh\necho \"{APP} {version}\"\n").into_bytes()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);
    for (name, data) in entries {
        let mut header = tar::Header::new_gnu();
        // Written into the raw name field so tests can build the unsafe
        // names `set_path` refuses.
        let field = &mut header.as_old_mut().name;
        field.fill(0);
        field[..name.len()].copy_from_slice(name.as_bytes());
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append(&header, *data).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The archive name this platform's `self update` asks for.
fn asset(opts: &SelfInstallOptions, version: &str) -> String {
    opts.asset_name(APP, version, &current_target())
}

fn release_archive(version: &str) -> Vec<u8> {
    let bin = binary_name();
    if current_target().contains("windows") {
        zip_bytes(&[(bin.as_str(), &script(version))])
    } else {
        tar_gz(&[(bin.as_str(), &script(version))])
    }
}

// ── sandbox ──────────────────────────────────────────────────────────────────

struct Sandbox {
    root: TempDir,
    env: InstallEnv,
}

impl Sandbox {
    fn new(version: &str) -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        let downloads = root.path().join("downloads");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&downloads).unwrap();
        let exe = downloads.join(binary_name());
        std::fs::write(&exe, script(version)).unwrap();
        let mut vars = BTreeMap::new();
        vars.insert("PATH".to_string(), String::new());
        let env = InstallEnv {
            app: APP.to_string(),
            version: version.to_string(),
            os: Os::current(),
            home: Some(home),
            vars,
            current_exe: exe,
            stdout_is_terminal: false,
            is_root: false,
            state_root: Some(root.path().join("state")),
            config_root: Some(root.path().join("config")),
            data_root: Some(root.path().join("data")),
            user_path_key: format!(
                "Software\\cli-framework-test\\{}",
                uuid::Uuid::new_v4().simple()
            ),
            uninstall_key_root: format!(
                "Software\\cli-framework-test\\uninstall-{}",
                uuid::Uuid::new_v4().simple()
            ),
            completion_command: None,
            self_invocation: format!("{APP} self"),
        };
        Sandbox { root, env }
    }

    /// Install the downloaded copy, then act as the installed binary.
    fn installed(version: &str, opts: &SelfInstallOptions) -> Self {
        let mut sb = Self::new(version);
        let req = InstallRequest {
            no_modify_path: true,
            ..Default::default()
        };
        let out = install(&sb.env, opts, &req).expect("install");
        sb.env.current_exe = out.binary_path;
        sb
    }

    fn binary(&self) -> PathBuf {
        self.env.current_exe.clone()
    }

    fn receipt(&self) -> InstallReceipt {
        InstallReceipt::load(&receipt_path(&self.env).unwrap())
            .unwrap()
            .expect("receipt")
    }

    /// Everything in the bin dir, sorted.
    fn bin_dir_listing(&self) -> Vec<String> {
        let dir = self.binary().parent().unwrap().to_path_buf();
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
            let _ = hkcu.delete_subkey_all(&self.env.user_path_key);
            let _ = hkcu.delete_subkey_all(&self.env.uninstall_key_root);
        }
    }
}

// ── mirror ───────────────────────────────────────────────────────────────────

struct Release {
    version: &'static str,
    archive: Vec<u8>,
    /// Override the listed digest, to serve a tampered archive.
    listed_digest: Option<String>,
    signature: Option<String>,
}

impl Release {
    fn new(version: &'static str) -> Self {
        Self {
            version,
            archive: release_archive(version),
            listed_digest: None,
            signature: None,
        }
    }

    fn sums(&self, opts: &SelfInstallOptions) -> String {
        let digest = self
            .listed_digest
            .clone()
            .unwrap_or_else(|| sha256_hex(&self.archive));
        format!("{digest}  {}\n", asset(opts, self.version))
    }

    fn signed(mut self, opts: &SelfInstallOptions, keys: &minisign::KeyPair) -> Self {
        self.signature = Some(sign(&keys.sk, self.sums(opts).as_bytes()));
        self
    }
}

fn sign(sk: &minisign::SecretKey, data: &[u8]) -> String {
    minisign::sign(None, sk, std::io::Cursor::new(data), Some("test"), None)
        .unwrap()
        .into_string()
}

async fn mirror(
    latest: &str,
    releases: &[Release],
    opts_for_names: &SelfInstallOptions,
) -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "version": latest })))
        .mount(&server)
        .await;
    for r in releases {
        let base = format!("/v{}", r.version);
        Mock::given(method("GET"))
            .and(path(format!("{base}/{}", asset(opts_for_names, r.version))))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(r.archive.clone()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{base}/SHA256SUMS")))
            .respond_with(ResponseTemplate::new(200).set_body_string(r.sums(opts_for_names)))
            .mount(&server)
            .await;
        if let Some(sig) = &r.signature {
            Mock::given(method("GET"))
                .and(path(format!("{base}/SHA256SUMS.minisig")))
                .respond_with(ResponseTemplate::new(200).set_body_string(sig.clone()))
                .mount(&server)
                .await;
        }
    }
    server
}

fn no_policy() -> SelfUpdatePolicy {
    SelfUpdatePolicy::default()
}

fn to(target: &str) -> UpdateRequest {
    UpdateRequest {
        target: Some(target.to_string()),
        ..Default::default()
    }
}

// ── archives ─────────────────────────────────────────────────────────────────

#[test]
fn sums_accept_text_and_binary_forms() {
    let h = "a".repeat(64);
    let sums = format!("{h}  one.tar.gz\n{h} *two.zip\nnot a line\n");
    assert_eq!(expected_digest(&sums, "one.tar.gz"), Some(h.clone()));
    assert_eq!(expected_digest(&sums, "two.zip"), Some(h));
    assert_eq!(expected_digest(&sums, "three.zip"), None);
    assert_eq!(expected_digest("zz  one.tar.gz", "one.tar.gz"), None);
}

#[test]
fn checksum_mismatch_and_unlisted_assets_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.tar.gz");
    std::fs::write(&file, b"payload").unwrap();
    let good = format!("{}  a.tar.gz\n", sha256_hex(b"payload"));
    verify_checksum(&file, "a.tar.gz", &good).unwrap();
    let bad = format!("{}  a.tar.gz\n", sha256_hex(b"other"));
    assert!(matches!(
        verify_checksum(&file, "a.tar.gz", &bad),
        Err(ArchiveError::Mismatch { .. })
    ));
    assert!(matches!(
        verify_checksum(&file, "b.tar.gz", &good),
        Err(ArchiveError::NotListed { .. })
    ));
}

#[test]
fn entry_names_are_checked() {
    assert!(is_safe_entry("demoapp"));
    assert!(is_safe_entry("./demoapp"));
    assert!(is_safe_entry("demoapp-x86_64/demoapp"));
    for bad in [
        "",
        "/etc/passwd",
        "\\evil",
        "../evil",
        "a/../../evil",
        "a\\..\\evil",
        "C:evil",
        "C:\\evil",
    ] {
        assert!(!is_safe_entry(bad), "{bad:?} should be unsafe");
    }
}

#[test]
fn binary_is_extracted_from_top_level_or_one_directory_down() {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in [
        (
            "top.tar.gz",
            tar_gz(&[("README.md", b"hi"), ("demoapp", b"bin")]),
        ),
        ("nested.tar.gz", tar_gz(&[("demoapp-v1/demoapp", b"bin")])),
        ("top.zip", zip_bytes(&[("demoapp", b"bin")])),
        ("nested.zip", zip_bytes(&[("pkg/demoapp", b"bin")])),
    ] {
        let archive = dir.path().join(name);
        std::fs::write(&archive, bytes).unwrap();
        let out = dir.path().join(format!("out-{name}"));
        std::fs::create_dir_all(&out).unwrap();
        let bin = extract_binary(&archive, "demoapp", &out).unwrap();
        assert_eq!(std::fs::read(&bin).unwrap(), b"bin", "{name}");
    }
}

#[test]
fn unsafe_archives_are_rejected_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in [
        (
            "up.tar.gz",
            tar_gz(&[("demoapp", b"bin"), ("../evil", b"x")]),
        ),
        (
            "abs.tar.gz",
            tar_gz(&[("/tmp/evil", b"x"), ("demoapp", b"bin")]),
        ),
        (
            "up.zip",
            zip_bytes(&[("demoapp", b"bin"), ("../evil", b"x")]),
        ),
    ] {
        let archive = dir.path().join(name);
        std::fs::write(&archive, bytes).unwrap();
        let out = dir.path().join(format!("out-{name}"));
        std::fs::create_dir_all(&out).unwrap();
        let err = extract_binary(&archive, "demoapp", &out).unwrap_err();
        assert!(
            matches!(err, ArchiveError::UnsafeEntry { .. }),
            "{name}: {err}"
        );
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0, "{name}");
    }
    assert!(!dir.path().join("evil").exists());
}

#[test]
fn missing_binary_and_unknown_formats_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("a.tar.gz");
    std::fs::write(&archive, tar_gz(&[("other", b"x")])).unwrap();
    assert!(matches!(
        extract_binary(&archive, "demoapp", dir.path()),
        Err(ArchiveError::MissingBinary { .. })
    ));
    let odd = dir.path().join("a.rar");
    std::fs::write(&odd, b"x").unwrap();
    assert!(matches!(
        extract_binary(&odd, "demoapp", dir.path()),
        Err(ArchiveError::UnknownFormat(_))
    ));
}

// ── signatures ───────────────────────────────────────────────────────────────

/// minisign-verify's test vector, made by the minisign CLI (a prehashed
/// `ED` signature): what release pipelines produce verifies here.
#[test]
fn minisign_cli_signatures_verify() {
    let key = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    let sig = "untrusted comment: signature from minisign secret key\n\
        RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n\
        trusted comment: timestamp:1556193335\tfile:test\n\
        y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==";
    check_public_key(key).unwrap();
    verify_signature(b"test", sig, key).unwrap();
    assert!(matches!(
        verify_signature(b"tset", sig, key),
        Err(ArchiveError::BadSignature(_))
    ));
}

#[test]
fn prehashed_signatures_verify_and_tampering_is_caught() {
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let pk = keys.pk.to_base64();
    let sig = sign(&keys.sk, b"sums");
    verify_signature(b"sums", &sig, &pk).unwrap();
    assert!(verify_signature(b"sumz", &sig, &pk).is_err());
    assert!(verify_signature(b"sums", &sign(&other.sk, b"sums"), &pk).is_err());
    assert!(matches!(
        verify_signature(b"sums", &sig, "not-a-key"),
        Err(ArchiveError::BadPublicKey(_))
    ));
}

#[test]
fn options_reject_an_invalid_public_key() {
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let pk = keys.pk.to_base64();
    assert!(SelfInstallOptions::github("o/demoapp")
        .public_key(Some(&pk))
        .validate()
        .is_ok());
    assert!(SelfInstallOptions::github("o/demoapp")
        .public_key(Some("RWnope"))
        .validate()
        .is_err());
}

// ── channels and policy ──────────────────────────────────────────────────────

fn gh(tag: &str, prerelease: bool, draft: bool) -> GithubRelease {
    serde_json::from_value(json!({
        "tag_name": tag,
        "prerelease": prerelease,
        "draft": draft,
        "assets": [],
    }))
    .unwrap()
}

#[test]
fn channels_pick_the_right_release() {
    let all = vec![
        gh("v1.0.0", false, false),
        gh("v1.1.0", false, false),
        gh("v1.2.0-rc.1", false, false),
        gh("v1.3.0", true, false),
        gh("v2.0.0", false, true),
        gh("other-v9.0.0", false, false),
    ];
    let pick = |c: &Channel| pick_release(&all, "v", c).map(|(v, _)| v.to_string());
    assert_eq!(pick(&Channel::Stable).as_deref(), Some("1.1.0"));
    assert_eq!(pick(&Channel::Latest).as_deref(), Some("1.3.0"));
    assert_eq!(
        pick(&Channel::parse("v1.0.0").unwrap()).as_deref(),
        Some("1.0.0")
    );
    assert_eq!(pick(&Channel::parse("2.0.0").unwrap()), None);
    assert!(Channel::parse("newest").is_err());
}

fn policy(enforced: serde_json::Value) -> Policy {
    serde_json::from_value(json!({
        "contract_version": 1,
        "app": APP,
        "profile": "default",
        "policy_version": 1,
        "max_cache_age_secs": 3600,
        "stale_action": "warn",
        "enforced": enforced,
    }))
    .unwrap()
}

#[test]
fn policy_keys_are_read_from_the_enforced_tree() {
    let p = SelfUpdatePolicy::from_policy(&policy(json!({
        KEY_ENABLED: true,
        KEY_CHANNEL: "stable",
        "self_update.base_url": "https://mirror.example/demoapp",
        KEY_MINIMUM_VERSION: "v1.4.0",
    })));
    assert!(p.allows_update());
    assert_eq!(p.channel.as_deref(), Some("stable"));
    assert_eq!(
        p.base_url.as_deref(),
        Some("https://mirror.example/demoapp")
    );
    assert_eq!(p.minimum_version, Some(semver::Version::new(1, 4, 0)));
    assert!(p.malformed.is_empty());
}

#[test]
fn malformed_policy_keys_take_the_safe_reading() {
    let p = SelfUpdatePolicy::from_policy(&policy(json!({
        KEY_ENABLED: "no",
        KEY_CHANNEL: "nightly",
        "self_update.base_url": "http://mirror.example",
        KEY_MINIMUM_VERSION: 3,
    })));
    assert!(!p.allows_update());
    assert_eq!(p.channel, None);
    assert_eq!(p.base_url, None);
    assert_eq!(p.malformed.len(), 4);
}

// ── --system ─────────────────────────────────────────────────────────────────

#[test]
fn system_installs_refuse_conflicting_flags() {
    let sb = Sandbox::new("1.2.3");
    let opts = SelfInstallOptions::github("o/demoapp");
    for req in [
        InstallRequest {
            system: true,
            bin_dir: Some(sb.root.path().join("bin")),
            ..Default::default()
        },
        InstallRequest {
            system: true,
            unmanaged: true,
            ..Default::default()
        },
    ] {
        let err = install(&sb.env, &opts, &req).unwrap_err();
        assert!(matches!(err, SelfInstallError::BadRequest(_)), "{err}");
    }
}

#[test]
fn system_installs_never_touch_the_path() {
    let sb = Sandbox::new("1.2.3");
    let req = InstallRequest {
        system: true,
        ..Default::default()
    };
    let dir = system_bin_dir(&sb.env);
    assert!(matches!(
        path_decision(&sb.env, &req, &dir),
        PathDecision::Skip(_)
    ));
}

/// Unprivileged, `/usr/local/bin` is not writable: the error names the
/// elevated command instead of failing halfway.
#[cfg(unix)]
#[test]
fn system_installs_without_privilege_ask_for_elevation() {
    let sb = Sandbox::new("1.2.3");
    let dir = system_bin_dir(&sb.env);
    let writable = tempfile::tempfile_in(&dir).is_ok();
    if writable {
        eprintln!("skipped: {} is writable here", dir.display());
        return;
    }
    let req = InstallRequest {
        system: true,
        ..Default::default()
    };
    let err = install(&sb.env, &SelfInstallOptions::github("o/demoapp"), &req).unwrap_err();
    match err {
        SelfInstallError::NeedsElevation { command, .. } => {
            assert!(command.contains("install --system"), "{command}")
        }
        other => panic!("{other}"),
    }
}

// ── update and rollback (Unix: the test binaries are shell scripts) ──────────

#[cfg(unix)]
mod unix {
    use super::*;

    fn http_opts(server: &wiremock::MockServer) -> SelfInstallOptions {
        SelfInstallOptions::http(server.uri()).completions(false)
    }

    fn names() -> SelfInstallOptions {
        SelfInstallOptions::http("https://names.invalid")
    }

    fn version_of(path: &Path) -> String {
        probe_version(path).unwrap().to_string()
    }

    #[tokio::test]
    async fn update_replaces_the_binary_and_rollback_swaps_back() {
        let server = mirror("1.3.0", &[Release::new("1.3.0")], &names()).await;
        let opts = http_opts(&server);
        let sb = Sandbox::installed("1.2.3", &opts);

        let out = update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap();
        assert_eq!(out.action, UpdateAction::Updated);
        assert_eq!(
            (out.current.as_str(), out.target.as_str()),
            ("1.2.3", "1.3.0")
        );
        assert_eq!(version_of(&sb.binary()), "1.3.0");
        let prev = prev_path(&sb.binary());
        assert_eq!(out.previous.as_deref(), Some(prev.as_path()));
        assert_eq!(std::fs::read(&prev).unwrap(), script("1.2.3"));
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&prev).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0, ".prev must not be executable");
        }
        let r = sb.receipt();
        assert_eq!(r.version, "1.3.0");
        assert_eq!(r.previous_version.as_deref(), Some("1.2.3"));
        assert_eq!(r.channel, "stable");
        assert_eq!(
            sb.bin_dir_listing(),
            vec![APP.to_string(), format!("{APP}.prev")]
        );

        let back = rollback(&sb.env, &no_policy()).unwrap();
        assert_eq!((back.from.as_str(), back.to.as_str()), ("1.3.0", "1.2.3"));
        assert_eq!(version_of(&sb.binary()), "1.2.3");
        assert_eq!(std::fs::read(&prev).unwrap(), script("1.3.0"));
        let r = sb.receipt();
        assert_eq!(r.version, "1.2.3");
        assert_eq!(r.previous_version.as_deref(), Some("1.3.0"));

        // A second rollback rolls forward again.
        rollback(&sb.env, &no_policy()).unwrap();
        assert_eq!(version_of(&sb.binary()), "1.3.0");
        assert_eq!(
            sb.bin_dir_listing(),
            vec![APP.to_string(), format!("{APP}.prev")]
        );

        // Uninstall removes the kept copy too.
        let gone = uninstall(&sb.env, false).unwrap();
        assert!(gone.removed.contains(&prev));
        assert!(!prev.exists());
    }

    #[tokio::test]
    async fn up_to_date_and_check_leave_everything_alone() {
        let server = mirror("1.3.0", &[Release::new("1.3.0")], &names()).await;
        let opts = http_opts(&server);
        let sb = Sandbox::installed("1.3.0", &opts);
        let out = update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap();
        assert_eq!(out.action, UpdateAction::UpToDate);

        let sb = Sandbox::installed("1.2.3", &opts);
        let req = UpdateRequest {
            check: true,
            ..Default::default()
        };
        let out = update(&sb.env, &opts, &no_policy(), &req).await.unwrap();
        assert_eq!(out.action, UpdateAction::Available);
        assert_eq!(out.target, "1.3.0");
        assert_eq!(version_of(&sb.binary()), "1.2.3");
        assert_eq!(sb.bin_dir_listing(), vec![APP.to_string()]);

        // --force reinstalls the current version.
        let sb = Sandbox::installed("1.3.0", &opts);
        let req = UpdateRequest {
            force: true,
            ..Default::default()
        };
        let out = update(&sb.env, &opts, &no_policy(), &req).await.unwrap();
        assert_eq!(out.action, UpdateAction::Updated);
    }

    #[tokio::test]
    async fn a_tampered_archive_is_refused_and_nothing_changes() {
        let mut release = Release::new("1.3.0");
        release.listed_digest = Some(sha256_hex(b"something else"));
        let server = mirror("1.3.0", &[release], &names()).await;
        let opts = http_opts(&server);
        let sb = Sandbox::installed("1.2.3", &opts);
        let err = update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, SelfInstallError::Verify(ArchiveError::Mismatch { .. })),
            "{err}"
        );
        assert_eq!(version_of(&sb.binary()), "1.2.3");
        assert_eq!(sb.bin_dir_listing(), vec![APP.to_string()]);
        assert_eq!(sb.receipt().version, "1.2.3");
    }

    #[tokio::test]
    async fn a_binary_reporting_another_version_is_refused() {
        let mut release = Release::new("1.3.0");
        release.archive = release_archive("1.2.9");
        let server = mirror("1.3.0", &[release], &names()).await;
        let opts = http_opts(&server);
        let sb = Sandbox::installed("1.2.3", &opts);
        let err = update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, SelfInstallError::BadRelease(_)), "{err}");
        assert_eq!(version_of(&sb.binary()), "1.2.3");
    }

    #[tokio::test]
    async fn signed_releases_require_a_valid_signature() {
        let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let rogue = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let pk = keys.pk.to_base64();
        let server = mirror(
            "1.3.0",
            &[
                Release::new("1.3.0").signed(&names(), &keys),
                Release::new("1.4.0"),
                Release::new("1.5.0").signed(&names(), &rogue),
            ],
            &names(),
        )
        .await;
        let opts = http_opts(&server).public_key(Some(&pk));
        let sb = Sandbox::installed("1.2.3", &opts);

        let err = update(&sb.env, &opts, &no_policy(), &to("1.4.0"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, SelfInstallError::Verify(ArchiveError::Unsigned(_))),
            "{err}"
        );
        let err = update(&sb.env, &opts, &no_policy(), &to("1.5.0"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, SelfInstallError::Verify(ArchiveError::BadSignature(_))),
            "{err}"
        );
        assert_eq!(version_of(&sb.binary()), "1.2.3");

        let out = update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap();
        assert_eq!(out.target, "1.3.0");
        assert_eq!(version_of(&sb.binary()), "1.3.0");
    }

    #[tokio::test]
    async fn downgrades_need_an_explicit_version() {
        let server = mirror("1.0.0", &[Release::new("1.0.0")], &names()).await;
        let opts = http_opts(&server);
        let sb = Sandbox::installed("1.2.3", &opts);
        let err = update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, SelfInstallError::Downgrade { .. }), "{err}");
        let out = update(&sb.env, &opts, &no_policy(), &to("1.0.0"))
            .await
            .unwrap();
        assert_eq!(out.action, UpdateAction::Updated);
        assert_eq!(version_of(&sb.binary()), "1.0.0");
        // An explicit version does not change the channel the receipt follows.
        assert_eq!(sb.receipt().channel, "stable");
    }

    #[tokio::test]
    async fn policy_can_disable_pin_or_redirect_updates() {
        let server = mirror("1.3.0", &[Release::new("1.3.0")], &names()).await;
        let opts = http_opts(&server);
        let sb = Sandbox::installed("1.2.3", &opts);

        let disabled = SelfUpdatePolicy {
            enabled: Some(false),
            ..Default::default()
        };
        let err = update(&sb.env, &opts, &disabled, &UpdateRequest::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, SelfInstallError::PolicyRefused { key, .. } if key == KEY_ENABLED),
            "{err}"
        );
        assert!(matches!(
            rollback(&sb.env, &disabled),
            Err(SelfInstallError::PolicyRefused { .. })
        ));

        let floor = SelfUpdatePolicy {
            minimum_version: Some(semver::Version::new(2, 0, 0)),
            ..Default::default()
        };
        let err = update(&sb.env, &opts, &floor, &UpdateRequest::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, SelfInstallError::PolicyRefused { key, .. } if key == KEY_MINIMUM_VERSION),
            "{err}"
        );

        let stable_only = SelfUpdatePolicy {
            channel: Some("stable".into()),
            ..Default::default()
        };
        let err = update(&sb.env, &opts, &stable_only, &to("latest"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, SelfInstallError::PolicyRefused { .. }),
            "{err}"
        );

        // An enforced mirror replaces the app's own source.
        let elsewhere = SelfInstallOptions::http("https://unreachable.invalid").completions(false);
        let mirrored = SelfUpdatePolicy {
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let out = update(&sb.env, &elsewhere, &mirrored, &UpdateRequest::default())
            .await
            .unwrap();
        assert_eq!(out.action, UpdateAction::Updated);
    }

    #[tokio::test]
    async fn only_the_recorded_binary_updates_itself() {
        let server = mirror("1.3.0", &[Release::new("1.3.0")], &names()).await;
        let opts = http_opts(&server);

        let sb = Sandbox::new("1.2.3");
        let err = update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, SelfInstallError::NotManaged { .. }), "{err}");

        let mut sb = Sandbox::installed("1.2.3", &opts);
        sb.env.current_exe = sb.root.path().join("downloads").join(APP);
        let err = update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, SelfInstallError::ReceiptMismatch { .. }),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_held_lock_blocks_a_second_update() {
        let server = mirror("1.3.0", &[Release::new("1.3.0")], &names()).await;
        let opts = http_opts(&server);
        let sb = Sandbox::installed("1.2.3", &opts);
        let lock = sb.binary().parent().unwrap().join(format!(".{APP}.lock"));
        std::fs::write(&lock, b"1").unwrap();
        let err = update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, SelfInstallError::Locked { .. }), "{err}");
        std::fs::remove_file(&lock).unwrap();
        update(&sb.env, &opts, &no_policy(), &UpdateRequest::default())
            .await
            .unwrap();
        assert!(!lock.exists());
    }

    #[test]
    fn rollback_needs_a_kept_binary() {
        let opts = names();
        let sb = Sandbox::installed("1.2.3", &opts);
        assert!(matches!(
            rollback(&sb.env, &no_policy()),
            Err(SelfInstallError::NothingToRollBack { .. })
        ));
    }

    /// A directory holding a release archive and its `SHA256SUMS`, as an
    /// air-gapped person would copy them.
    fn offline_release(dir: &Path, version: &'static str, opts: &SelfInstallOptions) -> PathBuf {
        let r = Release::new(version);
        std::fs::create_dir_all(dir).unwrap();
        let archive = dir.join(asset(opts, version));
        std::fs::write(&archive, &r.archive).unwrap();
        std::fs::write(dir.join("SHA256SUMS"), r.sums(opts)).unwrap();
        archive
    }

    #[tokio::test]
    async fn from_installs_and_updates_without_a_network() {
        let opts = SelfInstallOptions::http("https://unreachable.invalid").completions(false);
        let mut sb = Sandbox::new("1.0.0");
        let archive = offline_release(&sb.root.path().join("usb"), "1.3.0", &opts);
        let req = InstallRequest {
            no_modify_path: true,
            from: Some(archive.clone()),
            ..Default::default()
        };
        let out = install(&sb.env, &opts, &req).unwrap();
        assert_eq!(out.version, "1.3.0");
        assert_eq!(version_of(&out.binary_path), "1.3.0");
        assert!(out.source_left_behind.is_none());
        assert!(archive.exists(), "the archive is the person's; it stays");
        sb.env.current_exe = out.binary_path;
        sb.env.version = "1.3.0".into();
        assert_eq!(sb.receipt().version, "1.3.0");
        assert_eq!(sb.bin_dir_listing(), vec![APP.to_string()]);

        let newer = offline_release(&sb.root.path().join("usb2"), "1.4.0", &opts);
        let req = UpdateRequest {
            from: Some(newer),
            ..Default::default()
        };
        let out = update(&sb.env, &opts, &no_policy(), &req).await.unwrap();
        assert_eq!(out.action, UpdateAction::Updated);
        assert_eq!(version_of(&sb.binary()), "1.4.0");

        // A tampered archive beside the sums is refused.
        let dir = sb.root.path().join("usb3");
        let bad = offline_release(&dir, "1.5.0", &opts);
        std::fs::write(&bad, release_archive("6.6.6")).unwrap();
        let req = UpdateRequest {
            from: Some(bad),
            ..Default::default()
        };
        let err = update(&sb.env, &opts, &no_policy(), &req)
            .await
            .unwrap_err();
        assert!(matches!(err, SelfInstallError::Verify(_)), "{err}");
        assert_eq!(version_of(&sb.binary()), "1.4.0");
    }

    #[tokio::test]
    async fn from_requires_a_signature_when_the_app_has_a_key() {
        let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let opts = SelfInstallOptions::http("https://unreachable.invalid")
            .completions(false)
            .public_key(Some(&keys.pk.to_base64()));
        let sb = Sandbox::new("1.0.0");
        let dir = sb.root.path().join("usb");
        let archive = offline_release(&dir, "1.3.0", &opts);
        let req = InstallRequest {
            no_modify_path: true,
            from: Some(archive),
            ..Default::default()
        };
        let err = install(&sb.env, &opts, &req).unwrap_err();
        assert!(
            matches!(err, SelfInstallError::Verify(ArchiveError::Unsigned(_))),
            "{err}"
        );
        let sums = std::fs::read(dir.join("SHA256SUMS")).unwrap();
        std::fs::write(dir.join("SHA256SUMS.minisig"), sign(&keys.sk, &sums)).unwrap();
        install(&sb.env, &opts, &req).unwrap();
    }

    #[tokio::test]
    async fn the_notice_reports_newer_releases_once_a_day() {
        let server = mirror("1.3.0", &[], &names()).await;
        let opts = http_opts(&server);
        let sb = Sandbox::installed("1.2.3", &opts);

        let line = notice_line(&sb.env, &opts, &no_policy()).await.unwrap();
        assert!(
            line.contains("1.3.0") && line.contains("demoapp self update"),
            "{line}"
        );

        // Answered from the cache: the mirror is not asked again.
        server.reset().await;
        let line = notice_line(&sb.env, &opts, &no_policy()).await.unwrap();
        assert!(line.contains("1.3.0"), "{line}");
        assert!(server.received_requests().await.unwrap().is_empty());

        let mut quiet = sb.env.clone();
        quiet
            .vars
            .insert("DEMOAPP_NO_UPDATE_CHECK".into(), "1".into());
        assert_eq!(notice_line(&quiet, &opts, &no_policy()).await, None);
        let mut ci = sb.env.clone();
        ci.vars.insert("CI".into(), "true".into());
        assert_eq!(notice_line(&ci, &opts, &no_policy()).await, None);

        // Not installed by `self install`: nothing to act on, no notice.
        let bare = Sandbox::new("1.2.3");
        assert_eq!(notice_line(&bare.env, &opts, &no_policy()).await, None);
    }

    #[tokio::test]
    async fn the_notice_stays_quiet_when_current_or_unreachable() {
        let server = mirror("1.2.3", &[], &names()).await;
        let opts = http_opts(&server);
        let sb = Sandbox::installed("1.2.3", &opts);
        assert_eq!(notice_line(&sb.env, &opts, &no_policy()).await, None);

        let opts = SelfInstallOptions::http("http://127.0.0.1:9").completions(false);
        let sb = Sandbox::installed("1.2.3", &opts);
        assert_eq!(notice_line(&sb.env, &opts, &no_policy()).await, None);
    }
}
