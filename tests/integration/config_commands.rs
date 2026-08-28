//! Integration tests for the built-in `config` command group (`config
//! show`/`manifest`/`profile`/`refresh` — spec 021, "Command surface") via
//! `CliTestHarness`, following `tests/integration/auth_commands.rs`'s direct
//! template: asserts exit codes, stdout, and stderr per the spec 015 stream
//! contract.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::auth::{AccessToken, AuthError, AuthenticatedHttpClient, TokenProvider};
use cli_framework::command::Command;
use cli_framework::config::managed::{now_epoch_secs, PolicyCache, PolicyCacheEntry, PolicyClient};
use cli_framework::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
use cli_framework::config::{ConfigOptions, InMemoryBackend, VersionedConfig};
use cli_framework::http_retry::RetryableHttpClient;
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::testkit::CliTestHarness;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Ctx;
impl AppContext for Ctx {}

/// Always returns the same fixed bearer token — `invalidate()` is a no-op,
/// so a retried request after a 401 gets the identical token back (models
/// "the retry doesn't help," per spec 021's failure-mapping wording).
struct FixedTokenProvider;
#[async_trait::async_trait]
impl TokenProvider for FixedTokenProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        Ok(AccessToken::new("tok".to_string(), None))
    }
    async fn invalidate(&self) {}
}

fn policy_client(mock: &MockServer, cache: PolicyCache) -> Arc<PolicyClient> {
    let http = Arc::new(AuthenticatedHttpClient::new(
        RetryableHttpClient::new(reqwest::Client::new()),
        Arc::new(FixedTokenProvider) as Arc<dyn TokenProvider>,
    ));
    Arc::new(PolicyClient::new(http, cache, mock.uri(), "myapp"))
}

fn seed_cache(cache: &PolicyCache, profile: &str, policy_version: u64, max_cache_age_secs: u64) {
    seed_cache_aged(cache, profile, policy_version, max_cache_age_secs, 0);
}

/// Like [`seed_cache`], but backdates `fetched_at_epoch_secs` by `age_secs`
/// so a small `max_cache_age_secs` can be made stale on demand.
fn seed_cache_aged(
    cache: &PolicyCache,
    profile: &str,
    policy_version: u64,
    max_cache_age_secs: u64,
    age_secs: u64,
) {
    let now = cli_framework::config::managed::now_epoch_secs();
    cache
        .write(&PolicyCacheEntry {
            policy: json!({
                "contract_version": 1,
                "app": "myapp",
                "profile": profile,
                "policy_version": policy_version,
                "max_cache_age_secs": max_cache_age_secs,
                "stale_action": "warn",
                "enforced": {},
                "recommended": {},
            }),
            etag: Some("\"seed\"".to_string()),
            fetched_at_epoch_secs: now.saturating_sub(age_secs),
        })
        .unwrap();
}

fn field(key: &str, kind: FieldKind, default: serde_json::Value) -> FieldManifest {
    FieldManifest {
        key: key.to_string(),
        kind,
        default: Some(default),
        label: None,
        description: None,
        group: None,
        scope: Scope::Machine,
        platforms: vec![],
        secret: false,
        local_only: false,
        protected: false,
        manageable: true,
        enforceable: true,
        restart_required: false,
        constraints: None,
    }
}

fn sample_manifest() -> ConfigManifest {
    ConfigManifest::new(
        "myapp",
        vec![
            field("greeting", FieldKind::Str, json!("hello")),
            field("theme", FieldKind::Str, json!("light")),
            field("retries", FieldKind::Int, json!(1)),
        ],
    )
}

// ── Auto-registration ────────────────────────────────────────────────────────

#[test]
fn config_group_is_not_registered_without_a_manifest() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .build(Ctx)
        .unwrap();
    let registry = app.command_registry();
    // `config` is a group node (like `auth`), not a leaf command, so its
    // presence is checked via `group_metadata_for` / `resolve` on a leaf
    // path — `get("config")` alone would always be `None` regardless of
    // registration, since `get` only ever looks at leaf commands.
    assert!(
        registry.get("config").is_none(),
        "an app that never calls with_config_manifest must not get a `config` group"
    );
    assert!(
        registry.group_metadata_for("config").is_none(),
        "the `config` group node itself must not be registered either"
    );
    assert!(registry
        .resolve(&cli_framework::spec::command_tree::CommandPath::new(&["config", "show"]).unwrap())
        .is_none());
}

#[test]
fn config_group_is_registered_once_a_manifest_is_declared() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .build(Ctx)
        .unwrap();
    let registry = app.command_registry();
    assert!(registry.group_metadata_for("config").is_some());
    for leaf in ["show", "manifest", "profile", "refresh"] {
        assert!(
            registry
                .resolve(
                    &cli_framework::spec::command_tree::CommandPath::new(&["config", leaf])
                        .unwrap()
                )
                .is_some(),
            "config {leaf} must be registered"
        );
    }
}

#[test]
fn config_group_auto_registration_defers_to_a_pre_existing_root_level_config_command() {
    // Mirrors the identical "'auth' command already registered" guard shape
    // in `AppBuilder::build` — a consumer that already owns a root-level
    // command literally named `config` must not have it clobbered by the
    // built-in group.
    let user_command = Command {
        id: Arc::from("config"),
        spec: Arc::new(CommandSpec {
            summary: "A user-defined root command that happens to be named `config`",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        visibility: None,
        meta: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    };

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .register_command(user_command)
        .unwrap()
        .build(Ctx)
        .unwrap();

    let registry = app.command_registry();
    // The user's leaf command survives...
    assert!(registry.get("config").is_some());
    // ...and the built-in group's leaves were never installed on top of it.
    assert!(registry.group_metadata_for("config").is_none());
}

// ── config show ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn config_show_json_reports_default_recommended_and_enforced_provenance() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "contract_version": 1,
            "app": "myapp",
            "profile": "developers",
            "policy_version": 1,
            "max_cache_age_secs": 3600,
            "stale_action": "warn",
            "enforced": { "retries": 9 },
            "recommended": { "theme": "dark" },
        })))
        .mount(&mock)
        .await;
    let cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
    let pc = policy_client(&mock, cache);
    // Prime the cache with a real fetch — `config show` only ever reads the
    // cache, never the network (see `PolicyClient::cached_policy`).
    pc.fetch().await.unwrap();

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h
        .run(&["myapp", "config", "show", "--format", "json"])
        .await;

    assert_eq!(out.exit_code(), 0, "stderr: {}", out.stderr());
    let value: serde_json::Value = serde_json::from_str(out.stdout()).unwrap();
    let entries = value.as_array().unwrap();

    let greeting = entries.iter().find(|e| e["field"] == "greeting").unwrap();
    assert_eq!(greeting["value"], json!("hello"));
    assert_eq!(greeting["layer"], json!("default"));
    assert_eq!(greeting["locked"], json!(false));

    let theme = entries.iter().find(|e| e["field"] == "theme").unwrap();
    assert_eq!(theme["value"], json!("dark"));
    assert_eq!(theme["layer"], json!("recommended"));
    assert_eq!(theme["locked"], json!(false));

    let retries = entries.iter().find(|e| e["field"] == "retries").unwrap();
    assert_eq!(retries["value"], json!(9));
    assert_eq!(retries["layer"], json!("enforced"));
    assert_eq!(retries["locked"], json!(true));
}

#[tokio::test]
async fn config_show_table_is_the_default_format_and_lists_every_field() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "show"]).await;

    assert_eq!(out.exit_code(), 0, "stderr: {}", out.stderr());
    assert!(out.stdout().contains("greeting"));
    assert!(out.stdout().contains("hello"));
    assert!(out.stdout().contains("default"));
    assert!(out.stdout().contains("Field"));
    assert!(out.stdout().contains("Locked"));
    // Not valid JSON — proves table, not json, is the default rendering.
    assert!(serde_json::from_str::<serde_json::Value>(out.stdout()).is_err());
}

#[tokio::test]
async fn config_show_config_file_layer_reflects_the_wired_config_store() {
    #[derive(Clone, Default, Serialize, Deserialize)]
    struct AppConfig {
        schema_version: u32,
        greeting: String,
        theme: String,
        retries: u32,
    }
    impl VersionedConfig for AppConfig {
        fn schema_version(&self) -> u32 {
            self.schema_version
        }
        fn set_schema_version(&mut self, version: u32) {
            self.schema_version = version;
        }
    }

    let backend = InMemoryBackend::with_bytes(
        json!({"schema_version": 1, "greeting": "from-file", "theme": "", "retries": 0})
            .to_string()
            .into_bytes(),
    );

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_config_backend(Arc::new(backend))
        .with_config::<AppConfig>(ConfigOptions::new(1))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h
        .run(&["myapp", "config", "show", "--format", "json"])
        .await;

    assert_eq!(out.exit_code(), 0, "stderr: {}", out.stderr());
    let value: serde_json::Value = serde_json::from_str(out.stdout()).unwrap();
    let greeting = value
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["field"] == "greeting")
        .unwrap();
    assert_eq!(greeting["value"], json!("from-file"));
    assert_eq!(greeting["layer"], json!("config_file"));
}

// Bug 3 fix: a corrupt/unreadable policy cache must not be silently presented
// to the user as "this machine isn't managed" — `config show` still proceeds
// on local/default values (correct behavior for a genuinely unmanaged app),
// but must visibly warn about *why* it fell back, distinct from silence.
#[tokio::test]
async fn config_show_with_corrupt_cache_still_shows_local_values_but_warns() {
    let mock = MockServer::start().await;
    let backend = Arc::new(InMemoryBackend::with_bytes(
        br#"{"policy": "not-a-policy-object", "fetched_at_epoch_secs": 1}"#.to_vec(),
    ));
    let pc = policy_client(&mock, PolicyCache::new(backend));

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "show"]).await;

    assert_eq!(out.exit_code(), 0, "stderr: {}", out.stderr());
    // Still shows local/default values — a cache error must not make the
    // whole command fail.
    assert!(out.stdout().contains("greeting"));
    assert!(out.stdout().contains("hello"));
    // But the warning is visible, not silent.
    assert!(
        out.stderr()
            .to_lowercase()
            .contains("policy cache unreadable"),
        "stderr must contain the cache-unreadable warning, got: {:?}",
        out.stderr()
    );
}

// ── config manifest ──────────────────────────────────────────────────────────

#[tokio::test]
async fn config_manifest_round_trips_the_registered_manifest() {
    let manifest = sample_manifest();
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(manifest.clone())
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "manifest"]).await;

    assert_eq!(out.exit_code(), 0, "stderr: {}", out.stderr());
    let round_tripped: ConfigManifest =
        serde_json::from_str(out.stdout()).expect("`config manifest` stdout must be valid JSON");
    assert_eq!(round_tripped, manifest);
}

// ── config profile ───────────────────────────────────────────────────────────

#[tokio::test]
async fn config_profile_reports_unmanaged_with_no_policy_client_at_all() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "profile"]).await;

    assert_eq!(out.exit_code(), 0);
    assert!(out.stdout().to_lowercase().contains("unmanaged"));
}

#[tokio::test]
async fn config_profile_reports_unmanaged_when_nothing_was_ever_fetched() {
    let mock = MockServer::start().await;
    let cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
    let pc = policy_client(&mock, cache);

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h
        .run(&["myapp", "config", "profile", "--format", "json"])
        .await;

    assert_eq!(out.exit_code(), 0);
    let v: serde_json::Value = serde_json::from_str(out.stdout()).unwrap();
    assert_eq!(v["managed"], json!(false));
}

#[tokio::test]
async fn config_profile_reports_the_real_profile_and_version_after_a_successful_fetch() {
    let mock = MockServer::start().await;
    let backend = Arc::new(InMemoryBackend::new());
    let cache = PolicyCache::new(backend);
    seed_cache(&cache, "developers", 4, 3600);
    let pc = policy_client(&mock, cache);

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);

    let out = h.run(&["myapp", "config", "profile"]).await;
    assert_eq!(out.exit_code(), 0);
    assert!(out.stdout().contains("developers"));
    assert!(out.stdout().contains('4'));

    let out_json = h
        .run(&["myapp", "config", "profile", "--format", "json"])
        .await;
    let v: serde_json::Value = serde_json::from_str(out_json.stdout()).unwrap();
    assert_eq!(v["managed"], json!(true));
    assert_eq!(v["profile"], json!("developers"));
    assert_eq!(v["policy_version"], json!(4));
}

// Bug 3 fix: a corrupt/unreadable cache is a genuinely different condition
// from "access denied" (CFG003) or a refresh failure — `config profile`
// must report the dedicated CFG004 code, not CFG003, so an operator debugging
// a machine isn't misled into thinking access was denied when the real
// problem is a corrupt on-disk cache file.
#[tokio::test]
async fn config_profile_with_a_corrupt_cache_reports_cfg004() {
    let mock = MockServer::start().await;
    let backend = Arc::new(InMemoryBackend::with_bytes(
        br#"{"policy": "not-a-policy-object", "fetched_at_epoch_secs": 1}"#.to_vec(),
    ));
    let pc = policy_client(&mock, PolicyCache::new(backend));

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "profile"]).await;

    assert_eq!(out.exit_code(), 1);
    out.assert_diagnostic_code("CFG004");
}

// ── config refresh ───────────────────────────────────────────────────────────

#[tokio::test]
async fn config_refresh_without_a_policy_client_reports_cfg002() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "refresh"]).await;

    assert_eq!(out.exit_code(), 1);
    out.assert_diagnostic_code("CFG002");
}

#[tokio::test]
async fn config_refresh_success_reports_fresh_profile_and_version() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "contract_version": 1,
            "app": "myapp",
            "profile": "developers",
            "policy_version": 7,
            "max_cache_age_secs": 3600,
            "stale_action": "warn",
            "enforced": {},
            "recommended": {},
        })))
        .mount(&mock)
        .await;
    let cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
    let pc = policy_client(&mock, cache);

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "refresh"]).await;

    assert_eq!(out.exit_code(), 0, "stderr: {}", out.stderr());
    assert!(out.stdout().to_lowercase().contains("refreshed"));
    assert!(out.stdout().contains("developers"));
    assert!(out.stdout().contains('7'));
}

#[tokio::test]
async fn config_refresh_falls_back_to_cache_on_server_error() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    let backend = Arc::new(InMemoryBackend::new());
    let cache = PolicyCache::new(backend);
    seed_cache(&cache, "developers", 2, 3600);
    let pc = policy_client(&mock, cache);

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h
        .run(&["myapp", "config", "refresh", "--format", "json"])
        .await;

    assert_eq!(
        out.exit_code(),
        0,
        "a fresh-enough cache fallback is not a failure"
    );
    let v: serde_json::Value = serde_json::from_str(out.stdout()).unwrap();
    assert_eq!(v["outcome"], json!("from_cache"));
    assert_eq!(v["profile"], json!("developers"));
    assert_eq!(v["stale"], json!(false));
}

#[tokio::test]
async fn config_refresh_fresh_cache_fallback_reports_plainly_in_text_format() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    let backend = Arc::new(InMemoryBackend::new());
    let cache = PolicyCache::new(backend);
    seed_cache(&cache, "developers", 2, 3600);
    let pc = policy_client(&mock, cache);

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "refresh"]).await;

    assert_eq!(out.exit_code(), 0);
    assert!(!out.stdout().to_uppercase().contains("STALE"));
    assert!(out.stdout().contains("developers"));
    assert!(out.stdout().to_lowercase().contains("unreachable"));
}

#[tokio::test]
async fn config_refresh_stale_cache_fallback_reports_stale_in_text_format() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    let backend = Arc::new(InMemoryBackend::new());
    let cache = PolicyCache::new(backend);
    // max_cache_age_secs = 10, seeded 1000s in the past: stale, but
    // stale_action = warn (`seed_cache_aged`'s fixed action) proceeds anyway.
    seed_cache_aged(&cache, "developers", 2, 10, 1000);
    let pc = policy_client(&mock, cache);

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "refresh"]).await;

    assert_eq!(out.exit_code(), 0);
    assert!(out.stdout().to_uppercase().contains("STALE"));
    assert!(out.stdout().contains("developers"));
}

#[tokio::test]
async fn config_refresh_unmanaged_on_404_reports_unmanaged_in_text_format() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock)
        .await;
    let backend = Arc::new(InMemoryBackend::new());
    let cache = PolicyCache::new(backend);
    seed_cache(&cache, "developers", 2, 3600);
    let pc = policy_client(&mock, cache);

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "refresh"]).await;

    assert_eq!(out.exit_code(), 0, "unmanaged is a successful outcome");
    assert!(out.stdout().to_lowercase().contains("unmanaged"));
}

#[tokio::test]
async fn config_refresh_unreachable_with_no_cache_reports_cfg003() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    // No cache seeded at all — a server error with nothing to fall back to
    // is `PolicyClientError::Unreachable`, a real refresh failure.
    let pc = policy_client(&mock, PolicyCache::new(Arc::new(InMemoryBackend::new())));

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "refresh"]).await;

    assert_eq!(out.exit_code(), 1);
    out.assert_diagnostic_code("CFG003");
}

/// The negative case that matters most (spec 021): a `401` where the
/// `TokenProvider` retry also fails to help must **not** fall back to a
/// cached policy, even when a perfectly usable cache exists. This asserts
/// both the command's observable behavior (exit 1, no cached profile name
/// printed) and, directly, that the on-disk cache entry is untouched.
#[tokio::test]
async fn config_refresh_denied_does_not_fall_back_to_cache_and_fails() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;
    let backend = Arc::new(InMemoryBackend::new());
    let cache = PolicyCache::new(backend.clone());
    seed_cache(&cache, "cached-profile-should-not-appear", 2, 3600);
    let inspect_cache = PolicyCache::new(backend);
    let pc = policy_client(&mock, cache);

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "refresh"]).await;

    assert_eq!(out.exit_code(), 1);
    out.assert_diagnostic_code("CFG003");
    assert!(
        !out.stdout().contains("cached-profile-should-not-appear"),
        "Denied must never surface a cached profile; stdout: {:?}",
        out.stdout()
    );

    // The cache entry itself must be exactly what was seeded — Denied never
    // reads *or* writes it.
    let still_cached = inspect_cache.read().unwrap().unwrap();
    assert_eq!(
        still_cached.policy["profile"],
        json!("cached-profile-should-not-appear")
    );
}

// ── Bug 4: the manifest/resolver/PolicyClient machinery reaching a real
// ── typed config value via AppBuilder::build_with_config / `config refresh`
// ─────────────────────────────────────────────────────────────────────────

#[derive(Clone, Default, Serialize, Deserialize)]
struct BuildWithConfigNoRegression {
    schema_version: u32,
    greeting: String,
}
impl VersionedConfig for BuildWithConfigNoRegression {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

/// No-regression guard: an app that calls `with_config::<T>()` but never
/// `with_config_manifest`/`with_policy_client` must get exactly what
/// `build_with_config` always returned — `store.current()` verbatim, no
/// resolver/manifest involvement at all — even though `config-managed` is
/// compiled into this very test binary.
#[test]
fn build_with_config_without_manifest_or_policy_client_is_byte_for_byte_unchanged() {
    let backend = InMemoryBackend::with_bytes(
        json!({"schema_version": 1, "greeting": "from-file"})
            .to_string()
            .into_bytes(),
    );

    let (_app, config) = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_backend(Arc::new(backend))
        .with_config::<BuildWithConfigNoRegression>(ConfigOptions::new(1))
        .build_with_config::<Ctx, BuildWithConfigNoRegression>(Ctx)
        .unwrap();

    assert_eq!(config.greeting, "from-file");
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct BuildWithConfigManaged {
    schema_version: u32,
    greeting: String,
}
impl VersionedConfig for BuildWithConfigManaged {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

/// The centerpiece fix: with BOTH a manifest and a policy client registered,
/// `build_with_config`'s returned typed value folds in the policy client's
/// **cached** enforced value — without any network call. The `PolicyClient`
/// here points at a port nothing listens on (`127.0.0.1:1`): if
/// `build_with_config` ever called `fetch()` instead of `cached_policy()`,
/// this would fail/hang, proving the synchronous builder path truly never
/// touches the network.
#[test]
fn build_with_config_folds_a_cached_enforced_value_with_no_network_call() {
    let policy_cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
    policy_cache
        .write(&PolicyCacheEntry {
            policy: json!({
                "contract_version": 1,
                "app": "myapp",
                "profile": "developers",
                "policy_version": 3,
                "max_cache_age_secs": 3600,
                "stale_action": "warn",
                "enforced": { "greeting": "org-mandated" },
                "recommended": {},
            }),
            etag: None,
            fetched_at_epoch_secs: now_epoch_secs(),
        })
        .unwrap();
    let http = Arc::new(AuthenticatedHttpClient::new(
        RetryableHttpClient::new(reqwest::Client::new()),
        Arc::new(FixedTokenProvider) as Arc<dyn TokenProvider>,
    ));
    let pc = Arc::new(PolicyClient::new(
        http,
        policy_cache,
        "http://127.0.0.1:1",
        "myapp",
    ));

    let cfg_backend = InMemoryBackend::with_bytes(
        json!({"schema_version": 1, "greeting": "local-value", "theme": "", "retries": 0})
            .to_string()
            .into_bytes(),
    );

    let (_app, config) = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .with_config_backend(Arc::new(cfg_backend))
        .with_config::<BuildWithConfigManaged>(ConfigOptions::new(1))
        .build_with_config::<Ctx, BuildWithConfigManaged>(Ctx)
        .unwrap();

    assert_eq!(
        config.greeting, "org-mandated",
        "build_with_config must fold in the cached enforced value"
    );
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct RefreshE2EConfig {
    schema_version: u32,
    greeting: String,
    theme: String,
    retries: u32,
}
impl VersionedConfig for RefreshE2EConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

/// `config refresh` end-to-end: after a fresh-enforced-value fetch, the
/// SAME running `ConfigStore` (reached via `App::config_store`, independent
/// of the command's own printed output) reflects the newly enforced value —
/// proving the command actually reaches the live store, not just prints a
/// message about what the server said.
#[tokio::test]
async fn config_refresh_actually_updates_the_live_store_not_just_prints_a_message() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "contract_version": 1,
            "app": "myapp",
            "profile": "developers",
            "policy_version": 5,
            "max_cache_age_secs": 3600,
            "stale_action": "warn",
            "enforced": { "greeting": "freshly-enforced" },
            "recommended": {},
        })))
        .mount(&mock)
        .await;
    let cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
    let pc = policy_client(&mock, cache);

    let cfg_backend = InMemoryBackend::with_bytes(
        json!({"schema_version": 1, "greeting": "local-value", "theme": "", "retries": 0})
            .to_string()
            .into_bytes(),
    );

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .with_config_backend(Arc::new(cfg_backend))
        .with_config::<RefreshE2EConfig>(ConfigOptions::new(1))
        .build(Ctx)
        .unwrap();

    let store = app.config_store::<RefreshE2EConfig>().unwrap();
    assert_eq!(
        store.current().greeting,
        "local-value",
        "sanity: before refresh, the store still reflects the local file"
    );

    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "refresh"]).await;
    assert_eq!(out.exit_code(), 0, "stderr: {}", out.stderr());

    assert_eq!(
        store.current().greeting,
        "freshly-enforced",
        "config refresh must push the fetched enforced value into the live store"
    );
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct RefreshFoldFailureConfig {
    schema_version: u32,
    retries: bool,
}
impl VersionedConfig for RefreshFoldFailureConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

/// `config refresh` when the fetch itself succeeds but folding the result
/// into the live store fails (here: `sample_manifest()` declares `retries`
/// as an integer, matching the server's `enforced` value, but the real
/// application struct declares it `bool` — a manifest/struct drift). Must
/// surface as a refresh failure (CFG003), not panic or silently report
/// success while leaving the store untouched.
#[tokio::test]
async fn config_refresh_reports_a_failure_when_folding_into_the_live_store_fails() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "contract_version": 1,
            "app": "myapp",
            "profile": "developers",
            "policy_version": 1,
            "max_cache_age_secs": 3600,
            "stale_action": "warn",
            "enforced": { "retries": 9 },
            "recommended": {},
        })))
        .mount(&mock)
        .await;
    let cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
    let pc = policy_client(&mock, cache);

    let cfg_backend = InMemoryBackend::with_bytes(
        json!({"schema_version": 1, "retries": false})
            .to_string()
            .into_bytes(),
    );

    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_config_manifest(sample_manifest())
        .with_policy_client(pc)
        .with_config_backend(Arc::new(cfg_backend))
        .with_config::<RefreshFoldFailureConfig>(ConfigOptions::new(1))
        .build(Ctx)
        .unwrap();
    let store = app.config_store::<RefreshFoldFailureConfig>().unwrap();

    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "config", "refresh"]).await;

    assert_eq!(out.exit_code(), 1);
    out.assert_diagnostic_code("CFG003");
    assert!(
        !store.current().retries,
        "a failed fold must not partially mutate the running store"
    );
}
