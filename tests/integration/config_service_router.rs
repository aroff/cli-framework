//! `config_service_router` as an HTTP surface — spec 022's own testing
//! decision: "Good tests exercise the router as an HTTP surface... because
//! that is precisely what every client on every platform will depend on."
//!
//! Uses the real-listener pattern already established in this repo (see
//! `tests/integration/api_server_versioning.rs`) rather than socket-free
//! `tower::ServiceExt::oneshot` invocation: the socket-free path needs a
//! response-body-reading helper (`http-body-util`) that isn't a workspace
//! dependency today, and adding one purely for this test would be a bigger
//! footprint than reusing the already-proven real-listener helper, which
//! spec 022 explicitly allows as an alternative ("driven either by the
//! established real-listener helper or by direct service invocation
//! without a socket").
//!
//! `CallerIdentity` here is a controllable in-memory stub keyed by bearer
//! token, standing in for spec 022's "synthesized OIDC issuer helper set" —
//! it is used specifically to test *this crate's* auth middleware wiring
//! (a token in / claims out contract), not JWT verification itself (which
//! `cli-framework-oidc`'s own test suite already owns and which the
//! `with_config_service` example separately proves composes with a real
//! `OidcValidator`).

use async_trait::async_trait;
use cli_framework::config::service::{
    config_service_router, CallerIdentity, ConfigServiceError, ConfigServiceState, FsPolicyStore,
    InMemoryUserConfigStore,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct TokenIdentity {
    tokens: Mutex<HashMap<String, Value>>,
}

impl TokenIdentity {
    fn new() -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
        }
    }

    fn issue(&self, token: &str, claims: Value) {
        self.tokens
            .lock()
            .unwrap()
            .insert(token.to_string(), claims);
    }
}

#[async_trait]
impl CallerIdentity for TokenIdentity {
    async fn authenticate(
        &self,
        authorization_header: Option<&str>,
    ) -> Result<Value, ConfigServiceError> {
        let Some(header) = authorization_header else {
            return Err(ConfigServiceError::MissingCredential);
        };
        let Some(token) = header.strip_prefix("Bearer ") else {
            return Err(ConfigServiceError::InvalidCredential(
                "not a Bearer credential".to_string(),
            ));
        };
        self.tokens
            .lock()
            .unwrap()
            .get(token)
            .cloned()
            .ok_or_else(|| ConfigServiceError::InvalidCredential("unknown token".to_string()))
    }
}

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// The standard bundle every test in this file starts from: one app
/// (`myapp`), a `developers` profile (child of `base`), a `base` profile,
/// and an assignment rule mapping `realm_access.roles` containing
/// `developers` onto the `developers` profile.
fn write_standard_bundle(root: &Path) {
    write(
        &root.join("manifests/myapp.json"),
        r#"{
            "manifest_schema_version": 1,
            "app": "myapp",
            "fields": [
                {"key": "greeting", "kind": "string", "scope": "user"},
                {"key": "proxy_url", "kind": "url", "scope": "machine"},
                {"key": "api_key", "kind": "string", "scope": "user", "secret": true},
                {"key": "install_id", "kind": "string", "scope": "machine"}
            ]
        }"#,
    );
    write(
        &root.join("policies/myapp/base.toml"),
        r#"
        version = 1
        [enforced]
        "proxy_url" = "https://proxy.base.example.com"
        "#,
    );
    write(
        &root.join("policies/myapp/developers.toml"),
        r#"
        version = 7
        parent_profile = "base"
        max_cache_age_secs = 120
        stale_action = "refuse"

        [enforced]
        "install_id" = "should-not-actually-be-set-but-manifest-allows-machine-scope-here"

        [recommended]
        "greeting" = "hi developer"
        "#,
    );
    write(
        &root.join("assignments.toml"),
        r#"
        [myapp]
        [[myapp.rules]]
        claim_path = "realm_access.roles"
        operator = "contains"
        value = "developers"
        profile = "developers"
        "#,
    );
}

async fn spawn(root: &Path, identity: Arc<TokenIdentity>) -> String {
    let policy_store = Arc::new(FsPolicyStore::load(root).unwrap());
    let user_config_store = Arc::new(InMemoryUserConfigStore::new());
    let state = ConfigServiceState::new(policy_store, user_config_store, identity);
    state
        .validate_at_startup()
        .await
        .expect("bundle must validate");

    let router = config_service_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

fn developer_claims() -> Value {
    json!({"sub": "alice", "realm_access": {"roles": ["developers"]}})
}

// ── Policy fetch ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn policy_fetch_returns_the_flattened_document_for_the_resolved_profile() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/v1/policy/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["profile"], "developers");
    // `policy_version` is a combined token over the *entire* resolved
    // chain (bug 1 fix: `combined_chain_version`, `src/config/service/inherit.rs`),
    // not simply the leaf profile's own stored version (`7`) — asserting an
    // exact value here would pin an internal hash. The dedicated
    // `etag_changes_when_only_an_ancestor_profile_changes` test below
    // covers the behavior this field actually needs to guarantee.
    assert!(body["policy_version"].is_u64());
    // Parent-only field present, child field overrides where they conflict.
    assert_eq!(
        body["enforced"]["proxy_url"],
        "https://proxy.base.example.com"
    );
    assert_eq!(body["recommended"]["greeting"], "hi developer");
}

#[tokio::test]
async fn policy_wire_document_has_no_representation_of_inheritance() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/v1/policy/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let top_level: std::collections::BTreeSet<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(|s| s.as_str())
        .collect();
    let expected: std::collections::BTreeSet<&str> = [
        "contract_version",
        "app",
        "profile",
        "policy_version",
        "max_cache_age_secs",
        "stale_action",
        "enforced",
        "recommended",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        top_level, expected,
        "no parent_profile or inheritance-chain field may leak onto the wire"
    );
}

#[tokio::test]
async fn etag_revalidation_returns_not_modified() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let first = client
        .get(format!("{base}/v1/policy/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    let etag = first
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let second = client
        .get(format!("{base}/v1/policy/myapp"))
        .bearer_auth("good")
        .header("If-None-Match", etag)
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 304);
}

#[tokio::test]
async fn unmanaged_application_returns_404_with_no_policy_data() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    // No roles at all -- no assignment rule matches, no default configured.
    identity.issue("good", json!({"sub": "bob"}));
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/v1/policy/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().await.unwrap();
    assert!(body.get("enforced").is_none());
    assert!(body.get("recommended").is_none());
}

#[tokio::test]
async fn unmanaged_application_that_does_not_exist_at_all_is_also_404() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/v1/policy/no-such-app"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// ── Manifest ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn manifest_fetch_returns_the_stored_manifest() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/v1/manifest/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["app"], "myapp");
}

#[tokio::test]
async fn manifest_fetch_for_an_unknown_app_is_404() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/v1/manifest/no-such-app"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// ── Resolve diagnostic ───────────────────────────────────────────────────────

#[tokio::test]
async fn resolve_diagnostic_returns_profile_and_rule_with_no_configuration_values() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/v1/resolve/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["profile"], "developers");
    assert_eq!(body["matched_rule"]["claim_path"], "realm_access.roles");
    assert!(body.get("enforced").is_none());
    assert!(body.get("recommended").is_none());
}

// ── Roaming user config ──────────────────────────────────────────────────────

#[tokio::test]
async fn roaming_document_round_trips() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let get1 = client
        .get(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(get1.status(), 200);
    let etag0 = get1
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(etag0, "\"0\"");

    let put = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", etag0)
        .json(&json!({"greeting": "hi from device 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 200);

    let get2 = client
        .get(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    let body: Value = get2.json().await.unwrap();
    assert_eq!(body["greeting"], "hi from device 1");
}

#[tokio::test]
async fn a_stale_if_match_is_rejected_with_412_and_leaves_the_document_unchanged() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!({"greeting": "first write"}))
        .send()
        .await
        .unwrap();

    // Stale If-Match (still claims version 0, but the document is now at 1).
    let conflict = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!({"greeting": "conflicting write"}))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 412);

    let get = client
        .get(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    let body: Value = get.json().await.unwrap();
    assert_eq!(
        body["greeting"], "first write",
        "the conflicting write must not have landed"
    );
}

#[tokio::test]
async fn a_second_user_cannot_read_the_first_users_document() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("alice-token", json!({"sub": "alice"}));
    identity.issue("bob-token", json!({"sub": "bob"}));
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("alice-token")
        .header("If-Match", "\"0\"")
        .json(&json!({"greeting": "alice's secret preference"}))
        .send()
        .await
        .unwrap();

    let bob_get = client
        .get(format!("{base}/v1/config/myapp"))
        .bearer_auth("bob-token")
        .send()
        .await
        .unwrap();
    let body: Value = bob_get.json().await.unwrap();
    assert!(
        body.as_object().unwrap().is_empty(),
        "bob must not see alice's document"
    );
}

#[tokio::test]
async fn oversized_document_is_rejected() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let huge = "x".repeat(70 * 1024);
    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!({"greeting": huge}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
}

#[tokio::test]
async fn a_secret_field_in_a_write_is_rejected() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!({"api_key": "sk-should-never-roam"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn a_machine_scoped_field_in_a_write_is_rejected() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!({"install_id": "should-not-roam"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn a_write_with_no_if_match_header_is_rejected() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .json(&json!({"greeting": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ── Authentication, per endpoint ─────────────────────────────────────────────

async fn assert_endpoint_requires_auth(base: &str, method: reqwest::Method, path: &str) {
    let client = reqwest::Client::new();

    let no_token = client
        .request(method.clone(), format!("{base}{path}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        no_token.status(),
        401,
        "endpoint {path} must reject a request with no Authorization header"
    );

    let bad_token = client
        .request(method, format!("{base}{path}"))
        .bearer_auth("this-token-was-never-issued")
        .send()
        .await
        .unwrap();
    assert_eq!(
        bad_token.status(),
        401,
        "endpoint {path} must reject an invalid bearer token"
    );
}

#[tokio::test]
async fn every_endpoint_rejects_unauthenticated_and_invalid_token_requests() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    assert_endpoint_requires_auth(&base, reqwest::Method::GET, "/v1/policy/myapp").await;
    assert_endpoint_requires_auth(&base, reqwest::Method::GET, "/v1/manifest/myapp").await;
    assert_endpoint_requires_auth(&base, reqwest::Method::GET, "/v1/config/myapp").await;
    assert_endpoint_requires_auth(&base, reqwest::Method::PUT, "/v1/config/myapp").await;
    assert_endpoint_requires_auth(&base, reqwest::Method::GET, "/v1/resolve/myapp").await;
}

// ── Remaining edge cases: bad requests, unmanaged, and internal failures ────

#[tokio::test]
async fn resolve_diagnostic_for_an_unmanaged_identity_is_404() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", json!({"sub": "bob"}));
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/v1/resolve/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn a_validated_identity_missing_a_sub_claim_is_rejected_on_config_endpoints() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("no-sub", json!({"realm_access": {"roles": ["developers"]}}));
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let get_resp = client
        .get(format!("{base}/v1/config/myapp"))
        .bearer_auth("no-sub")
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), 401);

    let put_resp = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("no-sub")
        .header("If-Match", "\"0\"")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(put_resp.status(), 401);
}

#[tokio::test]
async fn an_unknown_field_in_a_write_is_rejected() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!({"this_field_does_not_exist": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn a_malformed_if_match_value_is_rejected() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "not-a-number")
        .json(&json!({"greeting": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn a_non_object_write_body_is_rejected() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!(["not", "an", "object"]))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn writing_to_an_app_with_no_manifest_is_404() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn(dir.path(), identity).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base}/v1/config/no-such-app"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// ── Internal storage failures map to 500, on every handler that can hit one ─

mod broken {
    use async_trait::async_trait;
    use cli_framework::config::service::{
        AssignmentRule, PolicyStore, StoreError, StoredManifest, StoredPolicy, StoredUserConfig,
        UserConfigStore, UserConfigWriteError,
    };
    use serde_json::{Map, Value};

    /// A `PolicyStore` where every method fails -- for exercising the
    /// handful of `internal_error(...)` (500) branches in `router.rs` a
    /// healthy `FsPolicyStore` never reaches.
    pub struct BrokenPolicyStore;

    #[async_trait]
    impl PolicyStore for BrokenPolicyStore {
        async fn manifest(&self, _app: &str) -> Result<Option<StoredManifest>, StoreError> {
            Err(StoreError::backend("manifest lookup broken"))
        }
        async fn policy(
            &self,
            _app: &str,
            _profile: &str,
        ) -> Result<Option<StoredPolicy>, StoreError> {
            Err(StoreError::backend("policy lookup broken"))
        }
        async fn policies_for_app(&self, _app: &str) -> Result<Vec<StoredPolicy>, StoreError> {
            Err(StoreError::backend("policies_for_app broken"))
        }
        async fn assignment_rules(&self, _app: &str) -> Result<Vec<AssignmentRule>, StoreError> {
            Err(StoreError::backend("assignment_rules broken"))
        }
        async fn apps(&self) -> Result<Vec<String>, StoreError> {
            Err(StoreError::backend("apps broken"))
        }
    }

    pub struct BrokenUserConfigStore;

    #[async_trait]
    impl UserConfigStore for BrokenUserConfigStore {
        async fn get(&self, _app: &str, _subject: &str) -> Result<StoredUserConfig, StoreError> {
            Err(StoreError::backend("user config get broken"))
        }
        async fn put(
            &self,
            _app: &str,
            _subject: &str,
            _doc: Map<String, Value>,
            _expected_version: u64,
        ) -> Result<u64, UserConfigWriteError> {
            Err(UserConfigWriteError::Store(StoreError::backend(
                "user config put broken",
            )))
        }
    }
}

async fn spawn_broken(identity: Arc<TokenIdentity>) -> String {
    let policy_store = Arc::new(broken::BrokenPolicyStore);
    let user_config_store = Arc::new(broken::BrokenUserConfigStore);
    let state = ConfigServiceState::new(policy_store, user_config_store, identity);
    // No `validate_at_startup()` here on purpose: a broken store can't be
    // validated either (it would fail on `apps()`), and that failure mode
    // is already covered by `src/config/service/validate.rs`'s own unit
    // tests -- this helper exists purely to reach the router's `500`
    // branches, which sit *after* startup validation would have already
    // run in a real deployment.
    let router = config_service_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn storage_failures_map_to_500_on_every_handler_that_touches_storage() {
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());
    let base = spawn_broken(identity).await;
    let client = reqwest::Client::new();

    let policy = client
        .get(format!("{base}/v1/policy/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(policy.status(), 500, "policy lookup failure must be a 500");

    let manifest = client
        .get(format!("{base}/v1/manifest/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(
        manifest.status(),
        500,
        "manifest lookup failure must be a 500"
    );

    let resolve = client
        .get(format!("{base}/v1/resolve/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resolve.status(),
        500,
        "resolve diagnostic failure must be a 500"
    );

    let get_config = client
        .get(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    assert_eq!(
        get_config.status(),
        500,
        "user config read failure must be a 500"
    );

    let put_config = client
        .put(format!("{base}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        put_config.status(),
        500,
        "user config write failure (manifest lookup for write validation) must be a 500"
    );
}

#[tokio::test]
async fn a_storage_failure_during_the_actual_write_after_validation_passes_is_also_a_500() {
    let dir = TempDir::new().unwrap();
    write_standard_bundle(dir.path());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", developer_claims());

    // A working PolicyStore (so manifest-based write validation succeeds)
    // paired with a UserConfigStore that always fails the actual write --
    // the one `internal_error` branch `storage_failures_map_to_500_on_every_handler`
    // above can't reach, since that test's `BrokenPolicyStore` fails the
    // manifest lookup before `put_user_config` ever calls `.put()`.
    let policy_store = Arc::new(FsPolicyStore::load(dir.path()).unwrap());
    let user_config_store = Arc::new(broken::BrokenUserConfigStore);
    let state = ConfigServiceState::new(policy_store, user_config_store, identity);
    let router = config_service_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let resp = client
        .put(format!("http://{addr}/v1/config/myapp"))
        .bearer_auth("good")
        .header("If-Match", "\"0\"")
        .json(&json!({"greeting": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
}

/// Bug 1 regression, exercised through the actual HTTP surface (spec 022's
/// own testing decision -- "Good tests exercise the router as an HTTP
/// surface"): a directly-mutable `PolicyStore` double, since there is no
/// admin write API yet (spec 023's job) to legitimately change a stored
/// policy's version out from under a running service.
mod mutable {
    use async_trait::async_trait;
    use cli_framework::config::service::{
        AssignmentRule, PolicyStore, RuleOperator, StoreError, StoredManifest, StoredPolicy,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MutablePolicyStore {
        policies: Mutex<HashMap<(String, String), StoredPolicy>>,
        assignments: Mutex<HashMap<String, Vec<AssignmentRule>>>,
    }

    impl MutablePolicyStore {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn set_policy(&self, policy: StoredPolicy) {
            self.policies
                .lock()
                .unwrap()
                .insert((policy.app.clone(), policy.profile.clone()), policy);
        }

        pub fn set_assignment_rules(&self, app: &str, rules: Vec<AssignmentRule>) {
            self.assignments
                .lock()
                .unwrap()
                .insert(app.to_string(), rules);
        }
    }

    #[async_trait]
    impl PolicyStore for MutablePolicyStore {
        async fn manifest(&self, _app: &str) -> Result<Option<StoredManifest>, StoreError> {
            Ok(None)
        }

        async fn policy(
            &self,
            app: &str,
            profile: &str,
        ) -> Result<Option<StoredPolicy>, StoreError> {
            Ok(self
                .policies
                .lock()
                .unwrap()
                .get(&(app.to_string(), profile.to_string()))
                .cloned())
        }

        async fn policies_for_app(&self, app: &str) -> Result<Vec<StoredPolicy>, StoreError> {
            Ok(self
                .policies
                .lock()
                .unwrap()
                .values()
                .filter(|p| p.app == app)
                .cloned()
                .collect())
        }

        async fn assignment_rules(&self, app: &str) -> Result<Vec<AssignmentRule>, StoreError> {
            Ok(self
                .assignments
                .lock()
                .unwrap()
                .get(app)
                .cloned()
                .unwrap_or_default())
        }

        async fn apps(&self) -> Result<Vec<String>, StoreError> {
            let apps: std::collections::BTreeSet<String> = self
                .policies
                .lock()
                .unwrap()
                .keys()
                .map(|(app, _)| app.clone())
                .collect();
            Ok(apps.into_iter().collect())
        }
    }

    pub fn policy_with_greeting(
        profile: &str,
        parent: Option<&str>,
        version: u64,
        greeting: &str,
    ) -> StoredPolicy {
        let mut enforced = serde_json::Map::new();
        enforced.insert("greeting".to_string(), serde_json::json!(greeting));
        StoredPolicy {
            app: "myapp".to_string(),
            profile: profile.to_string(),
            enforced,
            recommended: serde_json::Map::new(),
            parent_profile: parent.map(str::to_string),
            max_cache_age_secs: 3600,
            stale_action: cli_framework::config::StaleAction::Warn,
            version,
        }
    }

    pub fn policy_with_no_fields(
        profile: &str,
        parent: Option<&str>,
        version: u64,
    ) -> StoredPolicy {
        StoredPolicy {
            app: "myapp".to_string(),
            profile: profile.to_string(),
            enforced: serde_json::Map::new(),
            recommended: serde_json::Map::new(),
            parent_profile: parent.map(str::to_string),
            max_cache_age_secs: 3600,
            stale_action: cli_framework::config::StaleAction::Warn,
            version,
        }
    }

    pub fn default_rule_to(profile: &str) -> AssignmentRule {
        AssignmentRule {
            app: "myapp".to_string(),
            ord: 0,
            claim_path: String::new(),
            operator: RuleOperator::Default,
            value: None,
            profile: profile.to_string(),
        }
    }
}

async fn spawn_with_policy_store(
    policy_store: Arc<dyn cli_framework::config::service::PolicyStore>,
    identity: Arc<TokenIdentity>,
) -> String {
    let user_config_store = Arc::new(InMemoryUserConfigStore::new());
    let state = ConfigServiceState::new(policy_store, user_config_store, identity);
    // No `validate_at_startup()`: this helper feeds `MutablePolicyStore`,
    // which never has a manifest -- startup validation would refuse it for
    // an unrelated reason (`MissingManifest`) that has nothing to do with
    // what this test observes.
    let router = config_service_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

/// Bug 1's direct proof at the HTTP layer: bumping *only* the parent's
/// stored version/content, with the child's own row completely untouched,
/// must (a) change the served `policy_version`/ETag and (b) be reflected in
/// the flattened body on the very next request -- not held back by a stale
/// cache entry keyed on the child's own version alone. Reverting the fix in
/// `src/config/service/state.rs`/`inherit.rs` and rerunning this test
/// reproduces the bug: the second response's ETag and body would be
/// identical to the first.
#[tokio::test]
async fn etag_changes_when_only_an_ancestor_profile_changes() {
    let store = Arc::new(mutable::MutablePolicyStore::new());
    store.set_policy(mutable::policy_with_greeting("parent", None, 1, "v1"));
    store.set_policy(mutable::policy_with_no_fields("child", Some("parent"), 7));
    store.set_assignment_rules("myapp", vec![mutable::default_rule_to("child")]);

    let identity = Arc::new(TokenIdentity::new());
    identity.issue("good", json!({"sub": "alice"}));
    let base = spawn_with_policy_store(store.clone(), identity).await;

    let client = reqwest::Client::new();
    let first = client
        .get(format!("{base}/v1/policy/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    let etag1 = first
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let body1: Value = first.json().await.unwrap();
    assert_eq!(body1["enforced"]["greeting"], "v1");

    // Bump only the parent's stored version and content -- the child's own
    // row (profile "child", version 7) is completely untouched.
    store.set_policy(mutable::policy_with_greeting("parent", None, 2, "v2"));

    let second = client
        .get(format!("{base}/v1/policy/myapp"))
        .bearer_auth("good")
        .send()
        .await
        .unwrap();
    let etag2 = second
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(
        etag1, etag2,
        "the served ETag must change when only an ancestor profile's version changes"
    );
    let body2: Value = second.json().await.unwrap();
    assert_eq!(
        body2["enforced"]["greeting"], "v2",
        "the flattened body must reflect the ancestor's new value, not a stale cache entry"
    );

    // The old ETag must no longer satisfy a conditional request -- if it
    // did, that would itself prove the cache/ETag never actually moved.
    let third = client
        .get(format!("{base}/v1/policy/myapp"))
        .bearer_auth("good")
        .header("If-None-Match", etag1)
        .send()
        .await
        .unwrap();
    assert_ne!(
        third.status(),
        reqwest::StatusCode::NOT_MODIFIED,
        "a stale If-None-Match must not short-circuit to 304 once the ancestor has changed"
    );
}
