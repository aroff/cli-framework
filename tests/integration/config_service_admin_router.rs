//! The `/v1/admin/*` HTTP surface end to end (spec 023) — the mounted
//! router driven in-process (spec 022/023's shared seam), a `TokenIdentity`
//! double standing in for the synthesized-OIDC-issuer seam (the exact same
//! precedent `config_service_router.rs` already established — see that
//! file's own module docs for why a token-in/claims-out double is the right
//! level to test *this crate's* auth wiring at, not JWT verification
//! itself), and a real Postgres-backed `PgPolicyStore` doing double duty as
//! both `PolicyStore` (the device-facing read path) and `PolicyAdminStore`
//! (the admin write path) — so admin writes are actually observable on the
//! device-facing read path in the same process, which is exactly what
//! spec 023's testing decisions ask good tests here to assert against.
//!
//! Skips gracefully when `DATABASE_URL` is unset, same contract as every
//! other config-service Postgres test in this crate.

use async_trait::async_trait;
use cli_framework::config::manifest::{
    ConfigManifest, FieldConstraints, FieldKind, FieldManifest, Scope,
};
use cli_framework::config::service::postgres::{connect_and_migrate, PgPolicyStore, PgPool};
use cli_framework::config::service::{
    config_service_router, default_admin_rule, CallerIdentity, ConfigServiceError,
    ConfigServiceState, ConfigServiceState as CSS, FsPolicyStore, InMemoryUserConfigStore,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

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

fn admin_claims(sub: &str) -> Value {
    json!({"sub": sub, "realm_access": {"roles": ["config-admin"]}})
}

fn non_admin_claims(sub: &str) -> Value {
    json!({"sub": sub, "realm_access": {"roles": ["developers"]}})
}

async fn pool_or_skip() -> Option<PgPool> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping config-service admin router HTTP suite: DATABASE_URL is not set. \
             CI always sets this; local dev usually doesn't, which is expected."
        );
        return None;
    };
    Some(
        connect_and_migrate(&url)
            .await
            .expect("connect + migrate against DATABASE_URL"),
    )
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple())
}

/// Spawns a real HTTP server with a Postgres-backed `PolicyStore` +
/// `PolicyAdminStore` (the same `PgPolicyStore` instance doing both jobs) so
/// admin writes and device-facing reads see the identical, live state.
async fn spawn(pool: PgPool, identity: Arc<TokenIdentity>) -> String {
    let store = Arc::new(PgPolicyStore::new(pool));
    let state: Arc<CSS> = ConfigServiceState::new(
        store.clone(),
        Arc::new(InMemoryUserConfigStore::new()),
        identity,
    )
    .with_admin_store(store);
    let router = config_service_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

fn field(key: &str, kind: FieldKind) -> FieldManifest {
    FieldManifest {
        key: key.to_string(),
        kind,
        default: None,
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

/// A manifest exercising every field-flag combination the seven validation-
/// rejection tests need: an ordinary field, a secret one, a local-only one,
/// an unmanageable one, a non-enforceable one, and an enum field (this
/// system's only mechanism for an actual value-set *constraint* — `min`/
/// `max`/`allowed_values` on [`FieldConstraints`] are advisory-only and
/// never enforced by the reused validator; see this file's
/// `put_policy_rejects_a_value_outside_an_enum_fields_allowed_values` test
/// for the full explanation of that judgment call).
fn full_manifest(app: &str) -> ConfigManifest {
    let mut count_field = field("count", FieldKind::Int);
    count_field.constraints = Some(FieldConstraints {
        min: Some(0.0),
        max: Some(100.0),
        allowed_values: None,
    });
    ConfigManifest::new(
        app,
        vec![
            field("greeting", FieldKind::Str),
            field("api_key", FieldKind::Str).tap_secret(),
            field("install_id", FieldKind::Str).tap_local_only(),
            field("readonly_field", FieldKind::Str).tap_unmanageable(),
            field("recommend_only", FieldKind::Str).tap_not_enforceable(),
            field(
                "mode",
                FieldKind::Enum {
                    values: vec!["warn".to_string(), "refuse".to_string()],
                },
            ),
            count_field,
        ],
    )
}

// Small builder-style helpers so `full_manifest` reads declaratively.
trait FieldTap {
    fn tap_secret(self) -> Self;
    fn tap_local_only(self) -> Self;
    fn tap_unmanageable(self) -> Self;
    fn tap_not_enforceable(self) -> Self;
}
impl FieldTap for FieldManifest {
    fn tap_secret(mut self) -> Self {
        self.secret = true;
        self
    }
    fn tap_local_only(mut self) -> Self {
        self.local_only = true;
        self
    }
    fn tap_unmanageable(mut self) -> Self {
        self.manageable = false;
        self
    }
    fn tap_not_enforceable(mut self) -> Self {
        self.enforceable = false;
        self
    }
}

async fn put_manifest(
    client: &reqwest::Client,
    base: &str,
    app: &str,
    token: &str,
    manifest: &ConfigManifest,
) {
    let resp = client
        .put(format!("{base}/v1/admin/manifest/{app}"))
        .bearer_auth(token)
        .header("If-Match", "\"0\"")
        .json(manifest)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "manifest publish must succeed: {:?}",
        resp.text().await
    );
}

async fn put_default_assignment(
    client: &reqwest::Client,
    base: &str,
    app: &str,
    token: &str,
    profile: &str,
) {
    let resp = client
        .put(format!("{base}/v1/admin/assignments/{app}"))
        .bearer_auth(token)
        .header("If-Match", "\"0\"")
        .json(&json!({"rules": [{"claim_path": "", "operator": "default", "profile": profile}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "assignment publish must succeed: {:?}",
        resp.text().await
    );
}

// ── Auth layering: 401 vs 403 on every admin endpoint ───────────────────────

async fn assert_401_then_403(
    client: &reqwest::Client,
    base: &str,
    method: reqwest::Method,
    path: &str,
    non_admin_token: &str,
) {
    let no_token = client
        .request(method.clone(), format!("{base}{path}"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_token.status(), 401, "{path}: missing token must be 401");

    let bad_token = client
        .request(method.clone(), format!("{base}{path}"))
        .bearer_auth("never-issued")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_token.status(), 401, "{path}: invalid token must be 401");

    let non_admin = client
        .request(method, format!("{base}{path}"))
        .bearer_auth(non_admin_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        non_admin.status(),
        403,
        "{path}: a valid token lacking the admin role must be 403, not 401 or a panic"
    );
}

#[tokio::test]
async fn every_admin_endpoint_distinguishes_401_from_403() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin401");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    identity.issue("dev", non_admin_claims("bob"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    use reqwest::Method;
    assert_401_then_403(
        &client,
        &base,
        Method::PUT,
        &format!("/v1/admin/manifest/{app}"),
        "dev",
    )
    .await;
    assert_401_then_403(
        &client,
        &base,
        Method::GET,
        &format!("/v1/admin/policy/{app}/base"),
        "dev",
    )
    .await;
    assert_401_then_403(
        &client,
        &base,
        Method::PUT,
        &format!("/v1/admin/policy/{app}/base"),
        "dev",
    )
    .await;
    assert_401_then_403(
        &client,
        &base,
        Method::PATCH,
        &format!("/v1/admin/policy/{app}/base"),
        "dev",
    )
    .await;
    assert_401_then_403(
        &client,
        &base,
        Method::GET,
        &format!("/v1/admin/policy/{app}/base/history"),
        "dev",
    )
    .await;
    assert_401_then_403(
        &client,
        &base,
        Method::POST,
        &format!("/v1/admin/policy/{app}/base/history/1/restore"),
        "dev",
    )
    .await;
    assert_401_then_403(
        &client,
        &base,
        Method::GET,
        &format!("/v1/admin/assignments/{app}"),
        "dev",
    )
    .await;
    assert_401_then_403(
        &client,
        &base,
        Method::PUT,
        &format!("/v1/admin/assignments/{app}"),
        "dev",
    )
    .await;
    assert_401_then_403(&client, &base, Method::GET, "/v1/admin/export", "dev").await;
    assert_401_then_403(&client, &base, Method::POST, "/v1/admin/import", "dev").await;
}

// ── Manifest publish, observed through the device-facing read path ─────────

#[tokio::test]
async fn put_manifest_is_observable_through_the_device_facing_manifest_endpoint() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-manifest");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    let resp = client
        .get(format!("{base}/v1/manifest/{app}"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["app"], app);
}

// ── Policy PUT / PATCH / read-back ──────────────────────────────────────────

#[tokio::test]
async fn a_partial_update_changes_the_addressed_field_and_leaves_every_sibling_untouched() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-patch-siblings");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    let put = client
        .put(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({
            "enforced": {"greeting": "hi", "mode": "warn"},
            "recommended": {},
            "max_cache_age_secs": 3600,
            "stale_action": "warn"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 200, "{:?}", put.text().await);

    let patch = client
        .patch(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"1\"")
        .json(&json!({"enforced": {"greeting": "hello"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(patch.status(), 200, "{:?}", patch.text().await);

    let get = client
        .get(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let body: Value = get.json().await.unwrap();
    assert_eq!(
        body["enforced"]["greeting"], "hello",
        "addressed field changed"
    );
    assert_eq!(
        body["enforced"]["mode"], "warn",
        "untouched sibling in the SAME tree must survive, read back via the WHOLE policy"
    );
}

#[tokio::test]
async fn a_null_in_a_patch_removes_the_key_verified_via_the_device_facing_read_path() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-patch-null");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    client
        .put(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({
            "enforced": {"greeting": "hi"},
            "recommended": {},
            "max_cache_age_secs": 3600,
            "stale_action": "warn"
        }))
        .send()
        .await
        .unwrap();

    // The assignment rule must be written AFTER the profile it targets
    // exists -- an assignment rule naming a profile with no stored policy is
    // itself a validation rejection (spec 023 §6), by design.
    put_default_assignment(&client, &base, &app, "admin", "base").await;

    client
        .patch(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"1\"")
        .json(&json!({"enforced": {"greeting": null}}))
        .send()
        .await
        .unwrap();

    let served = client
        .get(format!("{base}/v1/policy/{app}"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(served.status(), 200);
    let body: Value = served.json().await.unwrap();
    assert!(
        body["enforced"].get("greeting").is_none(),
        "a null-patched key must be entirely absent from the served document, got {body:?}"
    );
}

#[tokio::test]
async fn moving_a_field_from_recommended_to_enforced_is_one_request_observed_on_the_served_document(
) {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-patch-move");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    client
        .put(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({
            "enforced": {},
            "recommended": {"greeting": "hi"},
            "max_cache_age_secs": 3600,
            "stale_action": "warn"
        }))
        .send()
        .await
        .unwrap();
    put_default_assignment(&client, &base, &app, "admin", "base").await;

    let patch = client
        .patch(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"1\"")
        .json(&json!({
            "enforced": {"greeting": "hi"},
            "recommended": {"greeting": null}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(patch.status(), 200, "{:?}", patch.text().await);

    let served = client
        .get(format!("{base}/v1/policy/{app}"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let body: Value = served.json().await.unwrap();
    assert_eq!(body["enforced"]["greeting"], "hi");
    assert!(body["recommended"].get("greeting").is_none());
}

#[tokio::test]
async fn a_stale_if_match_on_policy_put_is_412_and_leaves_the_document_unchanged() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-stale-if-match");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    client
        .put(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({
            "enforced": {"greeting": "first"},
            "recommended": {},
            "max_cache_age_secs": 3600,
            "stale_action": "warn"
        }))
        .send()
        .await
        .unwrap();

    let conflict = client
        .put(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"") // stale -- the real version is now 1
        .json(&json!({
            "enforced": {"greeting": "should-not-land"},
            "recommended": {},
            "max_cache_age_secs": 3600,
            "stale_action": "warn"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 412);

    let get = client
        .get(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let body: Value = get.json().await.unwrap();
    assert_eq!(
        body["enforced"]["greeting"], "first",
        "the conflicting write must not have landed"
    );
}

// ── Validation rejections: one test each ────────────────────────────────────

async fn put_policy_body(
    client: &reqwest::Client,
    base: &str,
    app: &str,
    token: &str,
    enforced: Value,
) -> reqwest::Response {
    client
        .put(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth(token)
        .header("If-Match", "\"0\"")
        .json(&json!({
            "enforced": enforced,
            "recommended": {},
            "max_cache_age_secs": 3600,
            "stale_action": "warn"
        }))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn put_policy_rejects_an_unknown_field() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-val-unknown");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    let resp = put_policy_body(&client, &base, &app, "admin", json!({"nonexistent": "x"})).await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn put_policy_rejects_a_wrong_type_value() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-val-type");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    // "count" is declared FieldKind::Int; a string value is a type mismatch.
    let resp = put_policy_body(
        &client,
        &base,
        &app,
        "admin",
        json!({"count": "not-a-number"}),
    )
    .await;
    assert_eq!(resp.status(), 400);
}

/// This system's only genuine value-set *constraint* mechanism: an enum
/// field's declared `values` list. `FieldConstraints.min`/`max`/
/// `allowed_values` are documented (`src/config/resolution/resolver.rs`,
/// `constraints_are_carried_but_not_enforced_by_the_resolver`) as advisory
/// metadata the reused validator never checks — inventing enforcement for
/// them here would be exactly the "second copy of the rules" spec 023
/// explicitly forbids. An enum field's `values`, by contrast, IS part of
/// its `FieldKind` and IS checked by the reused `value_matches_kind` (as
/// `WarningReason::TypeMismatch` / `PolicyValidationError::TypeMismatch`),
/// so this is the honest way to exercise "constraint violation" using only
/// the reused validation pipeline, not a new rule.
#[tokio::test]
async fn put_policy_rejects_a_value_outside_an_enum_fields_allowed_values() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-val-constraint");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    let resp = put_policy_body(
        &client,
        &base,
        &app,
        "admin",
        json!({"mode": "not-a-valid-mode"}),
    )
    .await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn put_policy_rejects_a_secret_field() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-val-secret");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    let resp = put_policy_body(&client, &base, &app, "admin", json!({"api_key": "sk-x"})).await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn put_policy_rejects_a_local_only_field() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-val-localonly");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    let resp = put_policy_body(&client, &base, &app, "admin", json!({"install_id": "x"})).await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn put_policy_rejects_an_unmanageable_field() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-val-unmanageable");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    let resp = put_policy_body(
        &client,
        &base,
        &app,
        "admin",
        json!({"readonly_field": "x"}),
    )
    .await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn put_policy_rejects_an_inheritance_cycle() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-val-cycle");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    // "b" starts as a root (no parent) -- must exist before "a" can validly
    // name it as a parent.
    let b_resp = client
        .put(format!("{base}/v1/admin/policy/{app}/b"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({
            "enforced": {}, "recommended": {},
            "max_cache_age_secs": 3600, "stale_action": "warn"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(b_resp.status(), 200, "{:?}", b_resp.text().await);

    // "a" names parent "b" -- a valid two-node chain, no cycle yet.
    let a_resp = client
        .put(format!("{base}/v1/admin/policy/{app}/a"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({
            "enforced": {}, "recommended": {}, "parent_profile": "b",
            "max_cache_age_secs": 3600, "stale_action": "warn"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(a_resp.status(), 200, "{:?}", a_resp.text().await);

    // Re-pointing "b" to parent "a" now closes the cycle (a -> b -> a) --
    // must be rejected.
    let b_cycle_resp = client
        .put(format!("{base}/v1/admin/policy/{app}/b"))
        .bearer_auth("admin")
        .header("If-Match", "\"1\"")
        .json(&json!({
            "enforced": {}, "recommended": {}, "parent_profile": "a",
            "max_cache_age_secs": 3600, "stale_action": "warn"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(b_cycle_resp.status(), 400);
}

// ── Mutation log accounting ──────────────────────────────────────────────────

#[tokio::test]
async fn a_rejected_write_appends_zero_log_rows_and_an_accepted_one_appends_exactly_one() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-log-accounting");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    let rejected =
        put_policy_body(&client, &base, &app, "admin", json!({"nonexistent": "x"})).await;
    assert_eq!(rejected.status(), 400);

    let history_after_rejection = client
        .get(format!("{base}/v1/admin/policy/{app}/base/history"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let body: Value = history_after_rejection.json().await.unwrap();
    assert!(
        body["entries"].as_array().unwrap().is_empty(),
        "a rejected write must append zero log rows: {body:?}"
    );

    let accepted = put_policy_body(&client, &base, &app, "admin", json!({"greeting": "hi"})).await;
    assert_eq!(accepted.status(), 200);

    let history_after_accept = client
        .get(format!("{base}/v1/admin/policy/{app}/base/history"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let body: Value = history_after_accept.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "an accepted write must append exactly one log row"
    );
    assert_eq!(entries[0]["actor"], "alice");
}

#[tokio::test]
async fn history_returns_records_in_ascending_resulting_version_order() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-history-order");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    for (i, greeting) in ["v1", "v2", "v3"].iter().enumerate() {
        let resp = client
            .put(format!("{base}/v1/admin/policy/{app}/base"))
            .bearer_auth("admin")
            .header("If-Match", format!("\"{i}\""))
            .json(&json!({
                "enforced": {"greeting": greeting},
                "recommended": {},
                "max_cache_age_secs": 3600,
                "stale_action": "warn"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    let history = client
        .get(format!("{base}/v1/admin/policy/{app}/base/history"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let body: Value = history.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    let versions: Vec<u64> = entries
        .iter()
        .map(|e| e["resulting_version"].as_u64().unwrap())
        .collect();
    assert_eq!(versions, vec![1, 2, 3]);
}

// ── Restore ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn restore_produces_the_earlier_document_as_a_new_version_leaving_intervening_records_intact()
{
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-restore");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    for (i, greeting) in ["v1", "v2", "v3"].iter().enumerate() {
        client
            .put(format!("{base}/v1/admin/policy/{app}/base"))
            .bearer_auth("admin")
            .header("If-Match", format!("\"{i}\""))
            .json(&json!({
                "enforced": {"greeting": greeting},
                "recommended": {},
                "max_cache_age_secs": 3600,
                "stale_action": "warn"
            }))
            .send()
            .await
            .unwrap();
    }

    // Restore to version 1 ("v1"). No If-Match is sent -- restore does not
    // take one (spec 023 §7).
    let restore = client
        .post(format!(
            "{base}/v1/admin/policy/{app}/base/history/1/restore"
        ))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(restore.status(), 200, "{:?}", restore.text().await);

    let get = client
        .get(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let body: Value = get.json().await.unwrap();
    assert_eq!(
        body["enforced"]["greeting"], "v1",
        "restore must produce the earlier document"
    );
    assert_eq!(
        body["version"], 4,
        "restore is a NEW forward version, not a rewrite"
    );

    let history = client
        .get(format!("{base}/v1/admin/policy/{app}/base/history"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let hbody: Value = history.json().await.unwrap();
    let entries = hbody["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        4,
        "history is append-only -- the restore added a 4th record"
    );
    assert_eq!(
        entries[1]["resulting_document"]["enforced"]["greeting"], "v2",
        "the intervening v2 record is untouched"
    );
    assert_eq!(entries[3]["kind"], "policy_restore");
}

#[tokio::test]
async fn restoring_a_version_that_never_existed_is_404() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-restore-404");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!(
            "{base}/v1/admin/policy/{app}/base/history/999/restore"
        ))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// ── Assignments ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn assignments_round_trip_and_report_the_current_version_via_etag() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-assignments-http");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    let initial = client
        .get(format!("{base}/v1/admin/assignments/{app}"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(
        initial.headers().get("etag").unwrap().to_str().unwrap(),
        "\"0\""
    );

    // The target profile must exist before an assignment rule can name it.
    put_policy_body(&client, &base, &app, "admin", json!({"greeting": "hi"})).await;
    put_default_assignment(&client, &base, &app, "admin", "base").await;

    let after = client
        .get(format!("{base}/v1/admin/assignments/{app}"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(
        after.headers().get("etag").unwrap().to_str().unwrap(),
        "\"1\""
    );
    let body: Value = after.json().await.unwrap();
    assert_eq!(body["rules"][0]["profile"], "base");
    assert_eq!(body["rules"][0]["operator"], "default");
}

// ── Export / Import ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_bad_import_stores_nothing_seeded_state_is_untouched() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    let seeded_app = unique("admin-import-seed");
    put_manifest(
        &client,
        &base,
        &seeded_app,
        "admin",
        &full_manifest(&seeded_app),
    )
    .await;
    put_policy_body(
        &client,
        &base,
        &seeded_app,
        "admin",
        json!({"greeting": "seeded"}),
    )
    .await;

    // Build a deliberately-invalid tar bundle: a manifest and a policy that
    // references a field the manifest doesn't declare.
    let broken_app = unique("admin-import-broken");
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("manifests")).unwrap();
    std::fs::write(
        dir.path().join(format!("manifests/{broken_app}.json")),
        serde_json::to_string(&full_manifest(&broken_app)).unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("policies").join(&broken_app)).unwrap();
    std::fs::write(
        dir.path()
            .join("policies")
            .join(&broken_app)
            .join("base.toml"),
        "version = 1\n[enforced]\n\"ghost_field\" = \"x\"\n",
    )
    .unwrap();
    let tar_bytes = tar_up(dir.path());

    let import = client
        .post(format!("{base}/v1/admin/import"))
        .bearer_auth("admin")
        .header("content-type", "application/x-tar")
        .body(tar_bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(import.status(), 400, "{:?}", import.text().await);

    let seeded_check = client
        .get(format!("{base}/v1/admin/policy/{seeded_app}/base"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let body: Value = seeded_check.json().await.unwrap();
    assert_eq!(
        body["enforced"]["greeting"], "seeded",
        "unrelated, previously-seeded state must be untouched by a rejected import"
    );

    let broken_check = client
        .get(format!("{base}/v1/admin/policy/{broken_app}/base"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(
        broken_check.status(),
        404,
        "the invalid app must not have been stored at all"
    );
}

fn tar_up(root: &Path) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut buf);
        builder.append_dir_all(".", root).unwrap();
        builder.finish().unwrap();
    }
    buf
}

/// `GET /v1/admin/export` exports the *entire* configuration set (spec 023:
/// "export the whole configuration set as a bundle"), not just one app —
/// so every export/import test below needs its own dedicated, freshly
/// migrated `CREATE DATABASE` rather than the shared `workspace_test`
/// database `pool_or_skip()` connects to. That shared database is used
/// concurrently by every other test in this suite (and, by this repo's own
/// shared-checkout convention, potentially by other worktrees' test runs
/// too) — some of which deliberately create app fixtures that are invalid
/// by the admin write path's own validation rules (this file's own
/// `config_service_admin_postgres.rs` sibling does, on purpose, to exercise
/// `PolicyAdminStore` methods directly without going through HTTP
/// validation at all). Exporting *that* shared database and re-validating
/// the whole thing on import would spuriously fail on apps this test never
/// touched. A dedicated, empty database sidesteps the whole problem, and
/// happens to also be the most faithful test of what export/import is
/// actually *for* (spec 023: "seeding a new environment").
struct FreshDatabase {
    admin_pool: PgPool,
    db_name: String,
    pool: PgPool,
}

impl FreshDatabase {
    async fn create(database_url: &str) -> Self {
        let idx = database_url
            .rfind('/')
            .expect("DATABASE_URL must contain a path segment");
        let admin_pool = PgPool::connect(&format!("{}/postgres", &database_url[..idx]))
            .await
            .expect("connect to the admin/maintenance database");
        let db_name = format!("cfw023_admin_router_{}", Uuid::new_v4().simple());
        sqlx_core::raw_sql::raw_sql(&format!("CREATE DATABASE {db_name}"))
            .execute(&admin_pool)
            .await
            .expect("CREATE DATABASE");
        let pool = connect_and_migrate(&format!("{}/{db_name}", &database_url[..idx]))
            .await
            .expect("connect + migrate the fresh database");
        Self {
            admin_pool,
            db_name,
            pool,
        }
    }

    async fn drop(self) {
        let _ = sqlx_core::raw_sql::raw_sql(&format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.db_name
        ))
        .execute(&self.admin_pool)
        .await;
    }
}

#[tokio::test]
async fn export_then_import_reproduces_the_same_served_document() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let db = FreshDatabase::create(&url).await;
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(db.pool.clone(), identity).await;
    let client = reqwest::Client::new();

    let app = unique("admin-export-roundtrip");
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;
    put_policy_body(&client, &base, &app, "admin", json!({"greeting": "hello"})).await;
    put_default_assignment(&client, &base, &app, "admin", "base").await;

    let export = client
        .get(format!("{base}/v1/admin/export"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(export.status(), 200);
    assert_eq!(
        export.headers().get("content-type").unwrap(),
        "application/x-tar"
    );
    let tar_bytes = export.bytes().await.unwrap();

    // Import it right back into the SAME (now fully self-consistent) fresh
    // database -- proves the exported bundle round-trips through
    // `FsPolicyStore::load` and back into storage, and the served document
    // afterward is identical to what was exported.
    let import = client
        .post(format!("{base}/v1/admin/import"))
        .bearer_auth("admin")
        .header("content-type", "application/x-tar")
        .body(tar_bytes.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(import.status(), 200, "{:?}", import.text().await);

    let served = client
        .get(format!("{base}/v1/policy/{app}"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let body: Value = served.json().await.unwrap();
    assert_eq!(body["enforced"]["greeting"], "hello");
    assert_eq!(body["profile"], "base");

    db.drop().await;
}

/// Import into a genuinely EMPTY, separately-migrated database — the
/// literal "seeding a new environment" scenario spec 023 frames export/
/// import around, as distinct from
/// `export_then_import_reproduces_the_same_served_document`'s
/// reimport-into-the-same-database check above: here the SOURCE and TARGET
/// are two entirely separate databases.
#[tokio::test]
async fn export_from_one_deployment_imports_into_a_genuinely_empty_one() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping: DATABASE_URL not set");
        return;
    };
    let source_db = FreshDatabase::create(&url).await;
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let source_base = spawn(source_db.pool.clone(), identity.clone()).await;
    let client = reqwest::Client::new();

    let app = unique("admin-fresh-env");
    put_manifest(&client, &source_base, &app, "admin", &full_manifest(&app)).await;
    put_policy_body(
        &client,
        &source_base,
        &app,
        "admin",
        json!({"greeting": "from source"}),
    )
    .await;
    put_default_assignment(&client, &source_base, &app, "admin", "base").await;

    let export = client
        .get(format!("{source_base}/v1/admin/export"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    let tar_bytes = export.bytes().await.unwrap();

    // A second, entirely separate, empty database -- nothing seeded,
    // nothing else has ever written to it.
    let target_db = FreshDatabase::create(&url).await;
    let target_base = spawn(target_db.pool.clone(), identity).await;

    let import = client
        .post(format!("{target_base}/v1/admin/import"))
        .bearer_auth("admin")
        .header("content-type", "application/x-tar")
        .body(tar_bytes.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(import.status(), 200, "{:?}", import.text().await);

    let served = client
        .get(format!("{target_base}/v1/policy/{app}"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(served.status(), 200);
    let body: Value = served.json().await.unwrap();
    assert_eq!(body["enforced"]["greeting"], "from source");

    source_db.drop().await;
    target_db.drop().await;
}

// ── Cache/ETag: an admin change must not hide behind a stale 304 ───────────

#[tokio::test]
async fn a_devices_next_revalidation_after_an_admin_change_gets_the_new_version_not_304() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-revalidation");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;
    put_policy_body(&client, &base, &app, "admin", json!({"greeting": "v1"})).await;
    put_default_assignment(&client, &base, &app, "admin", "base").await;

    let first = client
        .get(format!("{base}/v1/policy/{app}"))
        .bearer_auth("admin")
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

    // An admin change via PATCH -- the device never re-fetches on its own;
    // this simulates the server-side change that must eventually be
    // observed.
    client
        .patch(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"1\"")
        .json(&json!({"enforced": {"greeting": "v2"}}))
        .send()
        .await
        .unwrap();

    let second = client
        .get(format!("{base}/v1/policy/{app}"))
        .bearer_auth("admin")
        .header("If-None-Match", &etag1)
        .send()
        .await
        .unwrap();
    assert_ne!(
        second.status(),
        reqwest::StatusCode::NOT_MODIFIED,
        "the next revalidation after an admin change must NOT be a 304"
    );
    assert_eq!(second.status(), 200);
    let etag2 = second
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(etag1, etag2);
    let body: Value = second.json().await.unwrap();
    assert_eq!(body["enforced"]["greeting"], "v2");
}

// ── Admin store not configured -> 500, not a panic ──────────────────────────

#[tokio::test]
async fn admin_routes_without_an_admin_store_configured_respond_500_not_panic() {
    let dir = tempfile::TempDir::new().unwrap();
    let fs_store = Arc::new(FsPolicyStore::load(dir.path()).unwrap());
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let state =
        ConfigServiceState::new(fs_store, Arc::new(InMemoryUserConfigStore::new()), identity);
    // No `.with_admin_store(...)` -- exactly the pre-spec-023 construction
    // shape every existing caller uses.
    let router = config_service_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base = format!("http://{addr}");

    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base}/v1/admin/manifest/myapp"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({"manifest_schema_version": 1, "app": "myapp", "fields": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
}

// ── Malformed request bodies -> 400 ─────────────────────────────────────────

#[tokio::test]
async fn put_manifest_with_a_malformed_body_is_400() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    let resp = client
        .put(format!("{base}/v1/admin/manifest/myapp"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({"this": "is not a manifest"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn put_policy_with_a_malformed_body_is_400() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-malformed-policy");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;

    // Missing the required `max_cache_age_secs`/`stale_action` fields.
    let resp = client
        .put(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({"enforced": "not even an object"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn patch_policy_with_a_non_object_body_is_400() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-patch-non-object");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;
    put_policy_body(&client, &base, &app, "admin", json!({"greeting": "hi"})).await;

    let resp = client
        .patch(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"1\"")
        .json(&json!(["not", "an", "object"]))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn patch_policy_rejects_a_wrong_type_parent_profile() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-patch-bad-parent");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;
    put_policy_body(&client, &base, &app, "admin", json!({"greeting": "hi"})).await;

    let resp = client
        .patch(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"1\"")
        .json(&json!({"parent_profile": 42}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn patch_policy_rejects_a_wrong_type_max_cache_age_secs() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-patch-bad-cache-age");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;
    put_policy_body(&client, &base, &app, "admin", json!({"greeting": "hi"})).await;

    let resp = client
        .patch(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"1\"")
        .json(&json!({"max_cache_age_secs": "not-a-number"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn patch_policy_rejects_a_wrong_value_stale_action() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-patch-bad-stale-action");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;
    put_policy_body(&client, &base, &app, "admin", json!({"greeting": "hi"})).await;

    let resp = client
        .patch(format!("{base}/v1/admin/policy/{app}/base"))
        .bearer_auth("admin")
        .header("If-Match", "\"1\"")
        .json(&json!({"stale_action": "not-a-real-action"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn put_assignments_rejects_an_unknown_operator() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let app = unique("admin-assignments-bad-op");
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();
    put_manifest(&client, &base, &app, "admin", &full_manifest(&app)).await;
    put_policy_body(&client, &base, &app, "admin", json!({"greeting": "hi"})).await;

    let resp = client
        .put(format!("{base}/v1/admin/assignments/{app}"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({"rules": [{"claim_path": "team", "operator": "startswith", "profile": "base"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn import_of_genuinely_corrupt_tar_bytes_is_400() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn(pool, identity).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/admin/import"))
        .bearer_auth("admin")
        .header("content-type", "application/x-tar")
        .body(b"not a tar archive at all".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

// ── PolicyAdminStore storage failures map to 500 ────────────────────────────

mod broken_admin {
    use async_trait::async_trait;
    use cli_framework::config::manifest::ConfigManifest;
    use cli_framework::config::service::{
        AdminWriteError, AssignmentRule, FsPolicyStore, MutationLogEntry, PolicyAdminStore,
        PolicyWrite, StoreError,
    };
    use serde_json::Value;

    /// Every method fails with a plain storage error -- for exercising the
    /// `AdminWriteError::Store` (500) branch of `admin_write_error_response`,
    /// which a healthy `PgPolicyStore` never reaches in these tests.
    pub struct BrokenAdminStore;

    #[async_trait]
    impl PolicyAdminStore for BrokenAdminStore {
        async fn put_manifest(
            &self,
            _app: &str,
            _doc: ConfigManifest,
            _actor: &str,
            _expected_version: u64,
        ) -> Result<u64, AdminWriteError> {
            Err(AdminWriteError::Store(StoreError::backend(
                "put_manifest broken",
            )))
        }

        async fn put_policy(
            &self,
            _app: &str,
            _profile: &str,
            _policy: PolicyWrite,
            _kind: cli_framework::config::service::MutationKind,
            _submitted: Value,
            _actor: &str,
            _expected_version: u64,
        ) -> Result<u64, AdminWriteError> {
            Err(AdminWriteError::Store(StoreError::backend(
                "put_policy broken",
            )))
        }

        async fn assignment_rules_version(&self, _app: &str) -> Result<u64, StoreError> {
            Err(StoreError::backend("assignment_rules_version broken"))
        }

        async fn put_assignment_rules(
            &self,
            _app: &str,
            _rules: Vec<AssignmentRule>,
            _actor: &str,
            _expected_version: u64,
        ) -> Result<u64, AdminWriteError> {
            Err(AdminWriteError::Store(StoreError::backend(
                "put_assignment_rules broken",
            )))
        }

        async fn policy_history(
            &self,
            _app: &str,
            _profile: &str,
        ) -> Result<Vec<MutationLogEntry>, StoreError> {
            Err(StoreError::backend("policy_history broken"))
        }

        async fn import_bundle(
            &self,
            _bundle: &FsPolicyStore,
            _actor: &str,
        ) -> Result<cli_framework::config::service::ImportSummary, AdminWriteError> {
            Err(AdminWriteError::Store(StoreError::backend(
                "import_bundle broken",
            )))
        }
    }
}

async fn spawn_with_broken_admin_store(pool: PgPool, identity: Arc<TokenIdentity>) -> String {
    let store = Arc::new(PgPolicyStore::new(pool));
    let state: Arc<CSS> =
        ConfigServiceState::new(store, Arc::new(InMemoryUserConfigStore::new()), identity)
            .with_admin_store(Arc::new(broken_admin::BrokenAdminStore));
    let router = config_service_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn admin_store_failures_map_to_500_on_every_write_and_read_that_touches_it() {
    let Some(pool) = pool_or_skip().await else {
        return;
    };
    let identity = Arc::new(TokenIdentity::new());
    identity.issue("admin", admin_claims("alice"));
    let base = spawn_with_broken_admin_store(pool, identity).await;
    let client = reqwest::Client::new();

    let put_manifest_resp = client
        .put(format!("{base}/v1/admin/manifest/myapp"))
        .bearer_auth("admin")
        .header("If-Match", "\"0\"")
        .json(&json!({"manifest_schema_version": 1, "app": "myapp", "fields": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(put_manifest_resp.status(), 500);

    let history_resp = client
        .get(format!("{base}/v1/admin/policy/myapp/base/history"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(history_resp.status(), 500);

    let assignments_get_resp = client
        .get(format!("{base}/v1/admin/assignments/myapp"))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(assignments_get_resp.status(), 500);

    let restore_resp = client
        .post(format!(
            "{base}/v1/admin/policy/myapp/base/history/1/restore"
        ))
        .bearer_auth("admin")
        .send()
        .await
        .unwrap();
    assert_eq!(restore_resp.status(), 500);

    let empty_dir = tempfile::TempDir::new().unwrap();
    let import_resp = client
        .post(format!("{base}/v1/admin/import"))
        .bearer_auth("admin")
        .header("content-type", "application/x-tar")
        .body(tar_up(empty_dir.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(import_resp.status(), 500);
}

// ── default_admin_rule sanity ───────────────────────────────────────────────

#[test]
fn default_admin_rule_targets_the_documented_claim_and_role() {
    let rule = default_admin_rule();
    assert_eq!(rule.claim_path, "realm_access.roles");
    assert_eq!(rule.value, Some(Value::String("config-admin".to_string())));
}
