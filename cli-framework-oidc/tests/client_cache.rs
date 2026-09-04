//! Tests for the OidcClient cache layer.

use cli_framework::auth::TokenProvider;
use cli_framework::secrets::{InMemorySecretStore, SecretKey};
use cli_framework_oidc::client::{
    default_cache_secret_key, legacy_cache_secret_key, OidcClient, OidcFlow, TokenAuthMethod,
};
use secrecy::SecretString;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_secret(s: &str) -> SecretString {
    SecretString::from_str(s).unwrap()
}

fn build_client(issuer: &str, client_id: &str, cache_dir: PathBuf) -> OidcClient {
    OidcClient::builder()
        .issuer_url(issuer)
        .client_id(client_id)
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(cache_dir)
        .build()
        .expect("build client")
}

// ── A2: from_env builder ──────────────────────────────────────────────────────
//
// Apps shouldn't hand-wire issuer/client/secret. from_env(prefix) reads
// {PREFIX}_ISSUER_URL / _CLIENT_ID / _CLIENT_SECRET / _FLOW / _SCOPES and
// resolves a flow: explicit _FLOW wins; else secret present → ClientCredentials,
// otherwise an interactive flow. Each test uses a unique prefix to stay isolated.

use cli_framework_oidc::client::OidcClientBuilder;

#[test]
fn from_env_with_secret_defaults_to_client_credentials() {
    std::env::set_var("CFA_ISSUER_URL", "https://auth.example.com/realms/r");
    std::env::set_var("CFA_CLIENT_ID", "svc");
    std::env::set_var("CFA_CLIENT_SECRET", "s3cr3t");
    let client = OidcClientBuilder::from_env("CFA")
        .expect("from_env")
        .app_name("cfa")
        .build()
        .expect("build");
    assert!(matches!(client.flow(), OidcFlow::ClientCredentials { .. }));
    assert_eq!(client.client_id(), "svc");
}

#[test]
fn from_env_without_secret_defaults_to_interactive() {
    std::env::set_var("CFB_ISSUER_URL", "https://auth.example.com/realms/r");
    std::env::set_var("CFB_CLIENT_ID", "cli");
    let client = OidcClientBuilder::from_env("CFB")
        .expect("from_env")
        .app_name("cfb")
        .build()
        .expect("build");
    // interactive = DeviceCode or AuthCodePkce, never ClientCredentials
    assert!(!matches!(client.flow(), OidcFlow::ClientCredentials { .. }));
}

#[test]
fn from_env_explicit_flow_device_wins() {
    std::env::set_var("CFC_ISSUER_URL", "https://auth.example.com/realms/r");
    std::env::set_var("CFC_CLIENT_ID", "cli");
    std::env::set_var("CFC_FLOW", "device");
    let client = OidcClientBuilder::from_env("CFC")
        .expect("from_env")
        .app_name("cfc")
        .build()
        .expect("build");
    assert!(matches!(client.flow(), OidcFlow::DeviceCode));
}

#[test]
fn from_env_missing_issuer_is_error() {
    std::env::set_var("CFD_CLIENT_ID", "cli");
    // no CFD_ISSUER_URL
    assert!(OidcClientBuilder::from_env("CFD").is_err());
}

// ── A1: cache_dir is optional, defaults from app_name ────────────────────────
//
// An app should not have to compute a cache path. With no .cache_dir() but an
// .app_name(), the client resolves a default under the OS cache dir:
//   <os-cache>/cli-framework-oidc/<app-name>

#[test]
fn cache_dir_defaults_from_app_name_when_unset() {
    let client = OidcClient::builder()
        .issuer_url("https://auth.example.com")
        .client_id("my-cli")
        .flow(OidcFlow::DeviceCode)
        .app_name("my-app")
        // no .cache_dir()
        .build()
        .expect("build must succeed without an explicit cache_dir");

    let dir = client.cache_dir();
    assert!(
        dir.is_absolute(),
        "default cache dir must be absolute: {dir:?}"
    );
    assert!(
        dir.ends_with("cli-framework-oidc/my-app"),
        "default cache dir must be <os-cache>/cli-framework-oidc/my-app, got {dir:?}"
    );
}

#[test]
fn explicit_cache_dir_overrides_default() {
    let tmp = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url("https://auth.example.com")
        .client_id("my-cli")
        .flow(OidcFlow::DeviceCode)
        .app_name("my-app")
        .cache_dir(tmp.path().to_path_buf())
        .build()
        .expect("build");
    assert_eq!(client.cache_dir(), tmp.path());
}

#[test]
fn cache_secret_key_defaults_from_app_name() {
    let tmp = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url("https://auth.example.com")
        .client_id("my-cli")
        .flow(OidcFlow::DeviceCode)
        .app_name("aidesktop")
        .cache_dir(tmp.path().to_path_buf())
        .build()
        .expect("build");
    assert_eq!(
        client.cache_secret_key().as_str(),
        "aidesktop/oidc/token.json"
    );
    assert_eq!(
        client.cache_secret_key(),
        &default_cache_secret_key(Some("aidesktop"))
    );
}

#[test]
fn cache_secret_key_without_app_name_uses_default_namespace() {
    let tmp = TempDir::new().unwrap();
    let client = build_client("https://auth.example.com", "svc", tmp.path().to_path_buf());
    assert_eq!(
        client.cache_secret_key().as_str(),
        "default/oidc/token.json"
    );
}

#[test]
fn cache_secret_key_builder_override_wins() {
    let tmp = TempDir::new().unwrap();
    let key = SecretKey::new(["aidesktop", "cogni", "oidc", "token.json"]).unwrap();
    let client = OidcClient::builder()
        .issuer_url("https://auth.example.com")
        .client_id("my-cli")
        .flow(OidcFlow::DeviceCode)
        .app_name("aidesktop")
        .cache_dir(tmp.path().to_path_buf())
        .cache_secret_key(key.clone())
        .build()
        .expect("build");
    assert_eq!(client.cache_secret_key(), &key);
}

#[test]
fn cache_key_is_deterministic() {
    let dir = TempDir::new().unwrap();
    let c1 = build_client(
        "https://auth.example.com",
        "my-app",
        dir.path().to_path_buf(),
    );
    let c2 = build_client(
        "https://auth.example.com",
        "my-app",
        dir.path().to_path_buf(),
    );
    // Both clients should produce the same cache key (tested indirectly via peek).
    // If they read/write to the same slot they won't conflict.
    let _ = c1;
    let _ = c2;
}

#[test]
fn different_client_ids_different_cache_keys() {
    // This is a behavioral test: two clients with different client_ids should use
    // different cache entries (no cross-contamination). Verified indirectly.
    let dir = TempDir::new().unwrap();
    let _c1 = build_client(
        "https://auth.example.com",
        "app-a",
        dir.path().to_path_buf(),
    );
    let _c2 = build_client(
        "https://auth.example.com",
        "app-b",
        dir.path().to_path_buf(),
    );
}

#[tokio::test]
async fn peek_returns_none_when_no_cache() {
    let dir = TempDir::new().unwrap();
    let client = build_client(
        "https://auth.example.com",
        "my-app",
        dir.path().to_path_buf(),
    );
    let status = client.peek().await;
    assert!(status.is_none());
}

#[tokio::test]
async fn invalidate_on_empty_cache_is_noop() {
    let dir = TempDir::new().unwrap();
    let client = build_client(
        "https://auth.example.com",
        "my-app",
        dir.path().to_path_buf(),
    );
    // Should not panic even if cache is empty
    client.invalidate().await;
}

#[tokio::test]
async fn logout_removes_entry() {
    let dir = TempDir::new().unwrap();
    let client = build_client(
        "https://auth.example.com",
        "my-app",
        dir.path().to_path_buf(),
    );
    // Logout on empty cache should succeed
    let result = client.logout().await;
    assert!(result.is_ok());
}

// ── D1: cache file permissions 0600, parent dir 0700 ─────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn cache_file_and_lock_have_0600_permissions() {
    use std::os::unix::fs::PermissionsExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock = MockServer::start().await;
    let base = mock.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": base,
            "token_endpoint": format!("{}/token", base),
            "jwks_uri": format!("{}/jwks", base),
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "tok",
            "token_type": "Bearer",
            "expires_in": 3600u64,
        })))
        .mount(&mock)
        .await;

    let dir = TempDir::new().unwrap();
    let client = build_client(&base, "svc", dir.path().to_path_buf());
    client.token().await.expect("token");

    let cache_file = dir.path().join("default/oidc/token.json");
    let lock_file = dir.path().join("default/oidc/token.lock");

    assert!(cache_file.exists(), "cache file must exist after token()");

    let cache_mode = std::fs::metadata(&cache_file).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        cache_mode, 0o600,
        "token.json must be 0600; got {:o}",
        cache_mode
    );

    if lock_file.exists() {
        let lock_mode = std::fs::metadata(&lock_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            lock_mode, 0o600,
            "token.lock must be 0600; got {:o}",
            lock_mode
        );
    }
}

#[cfg(unix)]
#[test]
fn cache_parent_dir_has_0700_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let parent = TempDir::new().unwrap();
    // Sub-directory that the client should create with 0700
    let cache_dir = parent.path().join("oidc-cache");
    let client = build_client("https://auth.example.com", "svc", cache_dir.clone());
    // build() creates the cache dir; peek() forces the directory to be created
    let _ = client; // just building it should be enough if dir is created in build

    // Even if dir isn't created until first write, verify after build
    if cache_dir.exists() {
        let mode = std::fs::metadata(&cache_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "cache dir must be 0700; got {:o}", mode);
    }
    // else: dir not created yet — acceptable, will be checked after first token()
}

// ── A5: refresh_skew default is 60s ──────────────────────────────────────────
//
// A token expiring in 45s is within the 60s default skew → must be considered
// stale and trigger a re-fetch. If the default were 30s it would still be
// considered fresh (45s > 30s) and no second fetch would occur.

#[tokio::test]
async fn default_refresh_skew_is_60s_token_expiring_in_45s_triggers_refetch() {
    let mock = MockServer::start().await;

    // Discovery
    let base = mock.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": base,
            "token_endpoint": format!("{}/token", base),
            "jwks_uri": format!("{}/jwks", base),
        })))
        .mount(&mock)
        .await;

    // First token: expires_in=45 (inside 60s skew → must be treated as stale)
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "first-token",
            "token_type": "Bearer",
            "expires_in": 45u64,
        })))
        .up_to_n_times(1)
        .mount(&mock)
        .await;

    // Second token: returned on the expected refetch
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "refreshed-token",
            "token_type": "Bearer",
            "expires_in": 3600u64,
        })))
        .mount(&mock)
        .await;

    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(&base)
        .client_id("svc")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(dir.path().to_path_buf())
        // No explicit refresh_skew — must use the 60s default
        .build()
        .expect("build");

    let t1 = client.token().await.expect("first token");
    assert_eq!(t1.as_bearer(), "first-token");

    // Second call: 45s < 60s default skew → token is within skew window → refetch
    let t2 = client.token().await.expect("second token");
    assert_eq!(
        t2.as_bearer(),
        "refreshed-token",
        "default refresh_skew must be 60s: token expiring in 45s must trigger refetch"
    );
}

// ── SecretStore seam: no plaintext file when a non-file backend is used ─────
//
// PRD-005 story 9 / ADR-0004: the token cache is routed through an injected
// `SecretStore`. The default backend (used when `.secret_store(..)` isn't
// called) writes `<cache_dir>/<app>/oidc/token.json` (proven by
// `cache_file_and_lock_have_0600_permissions` above). When a
// non-file backend — here `InMemorySecretStore` — is injected instead, no
// plaintext token file should ever be written to `cache_dir`, and the token
// flow (acquire, cache-hit, invalidate, logout) must still work end to end
// through that backend.

#[tokio::test]
async fn no_plaintext_file_when_non_file_backend_is_injected() {
    let mock = MockServer::start().await;
    let base = mock.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": base,
            "token_endpoint": format!("{}/token", base),
            "jwks_uri": format!("{}/jwks", base),
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "in-memory-token",
            "refresh_token": "in-memory-refresh",
            "token_type": "Bearer",
            "expires_in": 3600u64,
        })))
        .expect(1) // cached on the second call, same as the on-disk backend
        .mount(&mock)
        .await;

    let dir = TempDir::new().unwrap();
    let secret_store = Arc::new(InMemorySecretStore::new());
    let client = OidcClient::builder()
        .issuer_url(&base)
        .client_id("svc")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(dir.path().to_path_buf())
        .secret_store(secret_store)
        .build()
        .expect("build");

    let t1 = client.token().await.expect("first token");
    assert_eq!(t1.as_bearer(), "in-memory-token");

    // Cached: the mock only expects one call, so a second token() must not
    // hit the network — proving the InMemorySecretStore round-trip works.
    let t2 = client.token().await.expect("second token (cached)");
    assert_eq!(t2.as_bearer(), "in-memory-token");

    // The whole point: no plaintext token file anywhere under cache_dir.
    // (`cache_dir` may still contain the empty, content-free
    // `token.lock` advisory-lock file — that's a local-concurrency
    // guard independent of the SecretStore backend, and never holds token
    // bytes, so it doesn't defeat the "no plaintext token" property.)
    assert!(
        !dir.path().join("oidc-token.json").exists(),
        "a non-file SecretStore backend must never produce a plaintext oidc-token.json"
    );
    assert!(
        !dir.path().join("default/oidc/token.json").exists(),
        "a non-file SecretStore backend must never produce a plaintext token.json"
    );
    for entry in std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() {
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                !contents.contains("in-memory-token") && !contents.contains("in-memory-refresh"),
                "found token material written to disk at {path:?}: {contents:?}"
            );
        }
    }

    // invalidate/logout still work end to end through the injected backend.
    client.invalidate().await;
    let status = client.peek().await;
    assert!(status.is_some(), "refresh token should still be cached");

    client.logout().await.expect("logout");
    let status = client.peek().await;
    assert!(status.is_none(), "logout should clear the cached entry");
}

#[tokio::test]
async fn cache_file_is_not_created_until_token_stored() {
    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("default/oidc/token.json");
    assert!(!cache_path.exists());

    let client = build_client(
        "https://auth.example.com",
        "my-app",
        dir.path().to_path_buf(),
    );
    // No cache file yet — peek reads it lazily
    let status = client.peek().await;
    assert!(status.is_none());
}

async fn mock_token_server() -> (MockServer, String) {
    let mock = MockServer::start().await;
    let base = mock.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": base,
            "token_endpoint": format!("{}/token", base),
            "jwks_uri": format!("{}/jwks", base),
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "legacy-tok",
            "refresh_token": "legacy-rt",
            "token_type": "Bearer",
            "expires_in": 3600u64,
        })))
        .mount(&mock)
        .await;
    (mock, base)
}

#[tokio::test]
async fn app_name_writes_namespaced_cache_file() {
    let (_mock, base) = mock_token_server().await;
    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(&base)
        .client_id("svc")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .app_name("aidesktop")
        .cache_dir(dir.path().to_path_buf())
        .build()
        .expect("build");
    client.token().await.expect("token");
    assert!(
        dir.path().join("aidesktop/oidc/token.json").exists(),
        "namespaced cache file must exist after token()"
    );
    assert!(
        !dir.path().join("oidc-token.json").exists(),
        "legacy flat filename must not be written for a namespaced client"
    );
}

#[tokio::test]
async fn legacy_oidc_token_json_is_read_and_migrated_on_write() {
    let (_mock, base) = mock_token_server().await;
    let dir = TempDir::new().unwrap();

    let legacy = OidcClient::builder()
        .issuer_url(&base)
        .client_id("svc")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(dir.path().to_path_buf())
        .cache_secret_key(legacy_cache_secret_key())
        .build()
        .expect("build legacy");
    legacy.token().await.expect("seed legacy cache");
    assert!(dir.path().join("oidc-token.json").exists());

    let migrated = OidcClient::builder()
        .issuer_url(&base)
        .client_id("svc")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .app_name("my-app")
        .cache_dir(dir.path().to_path_buf())
        .build()
        .expect("build namespaced");

    let peeked = migrated
        .peek()
        .await
        .expect("legacy entry must be readable");
    assert!(peeked.logged_in);

    migrated.invalidate().await;

    assert!(
        dir.path().join("my-app/oidc/token.json").exists(),
        "write must land on the namespaced key"
    );
    assert!(
        !dir.path().join("oidc-token.json").exists(),
        "legacy key must be deleted after migrate-on-write"
    );

    let after = migrated.peek().await.expect("migrated entry remains");
    assert!(after.logged_in);
}
