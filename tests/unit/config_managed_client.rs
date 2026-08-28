//! [`PolicyClient`] — spec 021's failure-mapping table, one test per case,
//! plus cache freshness/staleness behavior. This is "the single most
//! safety-critical piece of this whole spec" (spec 021 preamble): a 401
//! where retry also fails must be *observably identical* to 403, and neither
//! may ever fall back to cache.

use cli_framework::auth::{AccessToken, AuthError, AuthenticatedHttpClient, TokenProvider};
use cli_framework::config::managed::{
    refresh_managed_config, PolicyCache, PolicyCacheEntry, PolicyClient, PolicyClientError,
    PolicyOutcome, RoamingClientError, RoamingConfigClient,
};
use cli_framework::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
use cli_framework::config::{
    ConfigFormat, ConfigHandle, ConfigStore, InMemoryBackend, Policy, StaleAction, VersionedConfig,
};
use cli_framework::http_retry::RetryableHttpClient;
use serde_json::{json, Map};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Test doubles ─────────────────────────────────────────────────────────────

/// Always returns the same fixed bearer token.
struct FixedTokenProvider(&'static str);

#[async_trait::async_trait]
impl TokenProvider for FixedTokenProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        Ok(AccessToken::new(self.0.to_string(), None))
    }
    async fn invalidate(&self) {}
}

/// First `token()` call returns `token-v1`; every subsequent call (i.e. after
/// `AuthenticatedHttpClient` invalidates on a 401) returns `token-v2`. Models
/// "the retry produces a *different*, working token."
struct RotatingTokenProvider {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl TokenProvider for RotatingTokenProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let tok = if n == 0 { "token-v1" } else { "token-v2" };
        Ok(AccessToken::new(tok.to_string(), None))
    }
    async fn invalidate(&self) {}
}

/// Every `token()` call returns `AuthError::NotAuthenticated` — models "the
/// [post-invalidate] attempt fails to produce a token at all," per spec
/// 021's failure-mapping wording, distinct from "the retried *request*
/// itself came back 401."
struct NeverAuthenticatedProvider;

#[async_trait::async_trait]
impl TokenProvider for NeverAuthenticatedProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        Err(AuthError::NotAuthenticated)
    }
    async fn invalidate(&self) {}
}

fn authed_client(provider: Arc<dyn TokenProvider>) -> Arc<AuthenticatedHttpClient> {
    Arc::new(AuthenticatedHttpClient::new(
        RetryableHttpClient::new(reqwest::Client::new()),
        provider,
    ))
}

fn cache() -> PolicyCache {
    PolicyCache::new(Arc::new(InMemoryBackend::new()))
}

fn sample_policy_json(
    policy_version: u64,
    max_cache_age_secs: u64,
    stale_action: &str,
) -> serde_json::Value {
    json!({
        "contract_version": 1,
        "app": "myapp",
        "profile": "developers",
        "policy_version": policy_version,
        "max_cache_age_secs": max_cache_age_secs,
        "stale_action": stale_action,
        "enforced": {},
        "recommended": {},
    })
}

fn client(mock: &MockServer, provider: Arc<dyn TokenProvider>, cache: PolicyCache) -> PolicyClient {
    PolicyClient::new(authed_client(provider), cache, mock.uri(), "myapp")
}

/// Populate `cache` as if a prior successful fetch happened `age_secs` ago.
fn seed_cache(
    cache: &PolicyCache,
    policy_version: u64,
    max_cache_age_secs: u64,
    stale_action: &str,
    age_secs: u64,
) {
    let now = cli_framework::config::managed::now_epoch_secs();
    cache
        .write(&PolicyCacheEntry {
            policy: sample_policy_json(policy_version, max_cache_age_secs, stale_action),
            etag: Some("\"seed-etag\"".to_string()),
            fetched_at_epoch_secs: now.saturating_sub(age_secs),
        })
        .unwrap();
}

// ── 200 / 304 ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fresh_200_response_is_applied_and_cached_with_its_etag() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_json(sample_policy_json(1, 3600, "warn")),
        )
        .mount(&mock)
        .await;

    let cache = cache();
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);
    let outcome = policy_client.fetch().await.unwrap();
    match outcome {
        PolicyOutcome::Fresh(policy) => assert_eq!(policy.policy_version, 1),
        other => panic!("expected Fresh, got {other:?}"),
    }
}

#[tokio::test]
async fn both_200_and_304_refresh_the_cache_fetch_time() {
    // "Both a 200 and a 304 refresh the fetch time" (spec 021) — asserted by
    // making a cache that would otherwise be judged stale survive a 304.
    // Two `PolicyCache` handles share one backend: one is moved into the
    // `PolicyClient`, the other stays in the test to inspect the result.
    let backend = Arc::new(InMemoryBackend::new());
    let inspect_cache = PolicyCache::new(backend.clone());
    // max_cache_age_secs = 10, but seeded 1000s in the past: stale unless the
    // 304 path refreshes fetched_at.
    seed_cache(&inspect_cache, 5, 10, "refuse", 1000);

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .and(header("if-none-match", "\"seed-etag\""))
        .respond_with(ResponseTemplate::new(304))
        .mount(&mock)
        .await;

    let policy_client = PolicyClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        PolicyCache::new(backend.clone()),
        mock.uri(),
        "myapp",
    );
    let outcome = policy_client.fetch().await.unwrap();
    match outcome {
        PolicyOutcome::Fresh(policy) => assert_eq!(policy.policy_version, 5),
        other => panic!("expected Fresh (304 re-served), got {other:?}"),
    }

    let entry = inspect_cache
        .read()
        .unwrap()
        .expect("cache must still hold the re-served entry");
    let age = cli_framework::config::managed::now_epoch_secs()
        .saturating_sub(entry.fetched_at_epoch_secs);
    assert!(
        age < 5,
        "fetch time must have been refreshed by the 304, age was {age}s"
    );
}

// ── Malformed responses on the 200/304 path ─────────────────────────────────

#[tokio::test]
async fn fresh_200_with_unparseable_json_body_is_invalid_response() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&mock)
        .await;

    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache());
    let err = policy_client.fetch().await.unwrap_err();
    assert!(matches!(err, PolicyClientError::InvalidResponse { .. }));
}

#[tokio::test]
async fn fresh_200_with_json_not_matching_policy_schema_is_invalid_response() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"not": "a policy"})))
        .mount(&mock)
        .await;

    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache());
    let err = policy_client.fetch().await.unwrap_err();
    assert!(matches!(err, PolicyClientError::InvalidResponse { .. }));
}

#[tokio::test]
async fn not_modified_with_no_cached_entry_is_invalid_response() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&mock)
        .await;

    // No `seed_cache` call: the client has never cached anything, yet the
    // server claims "not modified" — a malformed/buggy-server case.
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache());
    let err = policy_client.fetch().await.unwrap_err();
    assert!(matches!(err, PolicyClientError::InvalidResponse { .. }));
}

#[tokio::test]
async fn not_modified_refresh_with_corrupt_cached_policy_is_cache_corrupt() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&mock)
        .await;

    let cache = cache();
    cache
        .write(&PolicyCacheEntry {
            policy: json!({"not": "a policy"}),
            etag: None,
            fetched_at_epoch_secs: cli_framework::config::managed::now_epoch_secs(),
        })
        .unwrap();
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);
    let err = policy_client.fetch().await.unwrap_err();
    assert!(matches!(err, PolicyClientError::CacheCorrupt { .. }));
}

#[tokio::test]
async fn server_error_fallback_with_corrupt_cached_policy_is_cache_corrupt() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let cache = cache();
    cache
        .write(&PolicyCacheEntry {
            policy: json!({"not": "a policy"}),
            etag: None,
            fetched_at_epoch_secs: cli_framework::config::managed::now_epoch_secs(),
        })
        .unwrap();
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);
    let err = policy_client.fetch().await.unwrap_err();
    assert!(matches!(err, PolicyClientError::CacheCorrupt { .. }));
}

#[tokio::test]
async fn unexpected_http_status_is_invalid_response() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(201)) // 2xx but not 200/304
        .mount(&mock)
        .await;

    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache());
    let err = policy_client.fetch().await.unwrap_err();
    assert!(matches!(err, PolicyClientError::InvalidResponse { .. }));
}

// ── Bug 1: corrupt cache must not block a fresh fetch ───────────────────────

/// Bug fix regression: `PolicyClient::fetch()` used to propagate ANY cache
/// read/parse error immediately via `?`, meaning a single corrupted cache
/// file permanently blocked every subsequent `fetch()` call from ever
/// reaching the network — even for a fresh `200 OK` that doesn't need the
/// cache at all except for ETag revalidation (an absent ETag just means an
/// unconditional GET). A cache-read failure must degrade to "as if no cache
/// existed" instead.
#[tokio::test]
async fn fresh_200_with_corrupt_cache_still_succeeds_treating_cache_as_absent() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sample_policy_json(1, 3600, "warn")))
        .mount(&mock)
        .await;

    let backend = Arc::new(InMemoryBackend::with_bytes(b"not json at all".to_vec()));
    let policy_client = PolicyClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        PolicyCache::new(backend),
        mock.uri(),
        "myapp",
    );
    let outcome = policy_client.fetch().await.unwrap();
    match outcome {
        PolicyOutcome::Fresh(policy) => assert_eq!(policy.policy_version, 1),
        other => panic!("expected Fresh despite a corrupt cache, got {other:?}"),
    }
}

// ── Bug 2: a 4xx that isn't 401/403/404 must never fall back to cache ───────

/// Bug fix regression: `classify()` used to send every 4xx that wasn't
/// 401/403/404 into the same bucket as a genuine 5xx/network failure
/// (`HttpFailureClass::ServerOrNetwork`), which reaches `fall_back_to_cache`.
/// Spec 021's failure-mapping table only ever names cache fallback for
/// "server error or network failure... no HTTP response at all, or 5xx" —
/// nothing else. A `400 Bad Request` must be a hard error that leaves the
/// cache completely untouched, not a silent, indefinite cache fallback.
#[tokio::test]
async fn client_error_400_does_not_fall_back_to_cache_and_leaves_it_untouched() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&mock)
        .await;

    let backend = Arc::new(InMemoryBackend::new());
    let cache = PolicyCache::new(backend.clone());
    seed_cache(&cache, 42, 999_999, "warn", 1); // fresh, would happily serve if read
    let inspect_cache = PolicyCache::new(backend);
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);

    let err = policy_client.fetch().await.unwrap_err();
    assert!(
        matches!(err, PolicyClientError::ClientError { status: 400, .. }),
        "expected ClientError{{status: 400}}, got {err:?}"
    );

    let still_cached = inspect_cache
        .read()
        .unwrap()
        .expect("a 400 must never clear the cache either");
    assert_eq!(
        still_cached.policy["policy_version"],
        json!(42),
        "cache must be exactly what was seeded — a 400 must never read OR write it"
    );
}

/// Regression guard for the fix above: splitting "genuinely retryable" from
/// "hard 4xx error" inside `classify()` must not accidentally reclassify a
/// real 5xx (502, distinct from the pre-existing 500 coverage above) out of
/// `ServerOrNetwork` — it must still fall back to cache exactly as before.
#[tokio::test]
async fn server_error_502_still_falls_back_to_cache_after_the_client_error_split() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&mock)
        .await;

    let cache = cache();
    seed_cache(&cache, 7, 3600, "warn", 10); // well within max age
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);

    let outcome = policy_client.fetch().await.unwrap();
    match outcome {
        PolicyOutcome::FromCache { policy, stale } => {
            assert_eq!(policy.policy_version, 7);
            assert!(!stale);
        }
        other => panic!("expected FromCache(stale=false) for a 502, got {other:?}"),
    }
}

// ── 401 ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn unauthorized_where_retry_succeeds_proceeds_normally() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .and(header("authorization", "Bearer token-v1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .and(header("authorization", "Bearer token-v2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sample_policy_json(9, 3600, "warn")))
        .mount(&mock)
        .await;

    let provider = Arc::new(RotatingTokenProvider {
        calls: AtomicUsize::new(0),
    });
    let policy_client = client(&mock, provider, cache());
    let outcome = policy_client.fetch().await.unwrap();
    match outcome {
        PolicyOutcome::Fresh(policy) => assert_eq!(policy.policy_version, 9),
        other => panic!("expected Fresh after successful retry, got {other:?}"),
    }
}

/// The negative case that matters most (spec 021): a naive implementation
/// that falls back to cache on any 401 must fail this test.
#[tokio::test]
async fn unauthorized_where_retry_also_fails_does_not_read_cache() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;

    let cache = cache();
    seed_cache(&cache, 42, 999_999, "warn", 1); // fresh, would happily serve if read
    let provider = Arc::new(FixedTokenProvider("always-stale"));
    let policy_client = client(&mock, provider, cache);

    let outcome = policy_client.fetch().await.unwrap();
    assert_denied_never_reads_cache(outcome);
}

/// Distinct from the request-level 401 above: the *token re-acquisition
/// itself* fails (no HTTP request is ever made). Spec 021: "[if] that
/// attempt fails to produce a token... the client treats this identically
/// to Forbidden." Uses the shared assertion helper below to prove it really
/// is identical.
#[tokio::test]
async fn token_reacquisition_failure_is_also_denied_and_does_not_read_cache() {
    let mock = MockServer::start().await;
    // No mock mounted for the policy path at all — if this were ever hit,
    // wiremock would 404 on an unmatched request, which the assertion below
    // would also reject (an actual request happening at all is itself a bug
    // for a provider whose *first* token() call fails).
    let cache = cache();
    seed_cache(&cache, 42, 999_999, "warn", 1);
    let policy_client = client(&mock, Arc::new(NeverAuthenticatedProvider), cache);

    let outcome = policy_client.fetch().await.unwrap();
    assert_denied_never_reads_cache(outcome);
}

// ── 403 ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn forbidden_never_reads_cache() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&mock)
        .await;

    let cache = cache();
    seed_cache(&cache, 42, 999_999, "warn", 1);
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);

    let outcome = policy_client.fetch().await.unwrap();
    assert_denied_never_reads_cache(outcome);
}

/// Shared assertion helper (spec 021: "pinned by one shared assertion
/// helper rather than two copies that could drift apart") — used by every
/// 401-retry-fails and 403 test above. `outcome` must be `Denied`; there is
/// deliberately no cache-derived data available to compare against `Denied`,
/// since the whole point is that the cache is never consulted in the first
/// place.
fn assert_denied_never_reads_cache(outcome: PolicyOutcome) {
    assert_eq!(
        outcome,
        PolicyOutcome::Denied,
        "401-retry-failed and 403 must produce the exact same observable outcome"
    );
}

// ── 404 ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn not_found_runs_unmanaged_and_clears_an_existing_cache() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock)
        .await;

    let cache = cache();
    seed_cache(&cache, 1, 3600, "warn", 1);

    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);
    let outcome = policy_client.fetch().await.unwrap();
    assert_eq!(outcome, PolicyOutcome::Unmanaged);
    // Cache-actually-cleared is asserted precisely in
    // `cache_is_actually_empty_after_a_404` below (using a second handle
    // onto the same backend, which this test's `cache` value can't offer
    // once moved into `client()`).
}

#[tokio::test]
async fn cache_is_actually_empty_after_a_404() {
    let backend = Arc::new(InMemoryBackend::new());
    let cache_handle = PolicyCache::new(backend.clone());
    cache_handle
        .write(&PolicyCacheEntry {
            policy: sample_policy_json(1, 3600, "warn"),
            etag: None,
            fetched_at_epoch_secs: cli_framework::config::managed::now_epoch_secs(),
        })
        .unwrap();

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock)
        .await;
    let policy_client = PolicyClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        PolicyCache::new(backend.clone()),
        mock.uri(),
        "myapp",
    );
    policy_client.fetch().await.unwrap();

    assert!(
        cache_handle.read().unwrap().is_none(),
        "404 must clear the cache, not merely skip reading it"
    );
}

// ── 5xx / network failure -> cache fallback ─────────────────────────────────

#[tokio::test]
async fn server_error_with_fresh_cache_falls_back_to_cache() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let cache = cache();
    seed_cache(&cache, 7, 3600, "warn", 10); // well within max age
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);

    let outcome = policy_client.fetch().await.unwrap();
    match outcome {
        PolicyOutcome::FromCache { policy, stale } => {
            assert_eq!(policy.policy_version, 7);
            assert!(!stale);
        }
        other => panic!("expected FromCache(stale=false), got {other:?}"),
    }
}

#[tokio::test]
async fn stale_cache_with_warn_action_proceeds_and_reports_stale() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let cache = cache();
    seed_cache(&cache, 7, 10, "warn", 999); // far past max_cache_age_secs
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);

    let outcome = policy_client.fetch().await.unwrap();
    match outcome {
        PolicyOutcome::FromCache { stale, .. } => assert!(stale),
        other => panic!("expected FromCache(stale=true), got {other:?}"),
    }
}

#[tokio::test]
async fn stale_cache_with_refuse_action_fails_startup() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let cache = cache();
    seed_cache(&cache, 7, 10, "refuse", 999);
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache);

    let err = policy_client.fetch().await.unwrap_err();
    assert!(matches!(err, PolicyClientError::StaleCacheRefused { .. }));
}

#[tokio::test]
async fn network_failure_with_no_cache_surfaces_as_unreachable() {
    // 127.0.0.1:1 refuses connections outright (no server bound there) —
    // this is a genuine network failure, not an HTTP error response,
    // exercising the "no status at all" branch of the failure classifier
    // distinctly from the 5xx tests above.
    let policy_client = PolicyClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        cache(),
        "http://127.0.0.1:1",
        "myapp",
    );
    let err = policy_client.fetch().await.unwrap_err();
    assert!(matches!(err, PolicyClientError::Unreachable { .. }));
}

#[tokio::test]
async fn network_failure_with_fresh_cache_falls_back_to_cache() {
    let cache_handle = cache();
    seed_cache(&cache_handle, 3, 3600, "warn", 5);
    let policy_client = PolicyClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        cache_handle,
        "http://127.0.0.1:1",
        "myapp",
    );
    let outcome = policy_client.fetch().await.unwrap();
    match outcome {
        PolicyOutcome::FromCache { policy, stale } => {
            assert_eq!(policy.policy_version, 3);
            assert!(!stale);
        }
        other => panic!("expected FromCache, got {other:?}"),
    }
}

// ── Background refresh ───────────────────────────────────────────────────────

#[tokio::test]
async fn background_refresh_never_propagates_a_failure() {
    let policy_client = PolicyClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        cache(),
        "http://127.0.0.1:1",
        "myapp",
    );
    // Must not panic, and must report failure via its bool return rather
    // than an Err/panic — "leaves a running application untouched."
    let ok = policy_client.background_refresh().await;
    assert!(!ok);
}

#[tokio::test]
async fn background_refresh_reports_true_on_success() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sample_policy_json(1, 3600, "warn")))
        .mount(&mock)
        .await;
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache());
    assert!(policy_client.background_refresh().await);
}

// ── cached_policy (no network — spec 021 "Command surface" seam) ────────────

#[tokio::test]
async fn cached_policy_with_nothing_cached_yet_is_none() {
    let mock = MockServer::start().await;
    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache());
    assert_eq!(policy_client.cached_policy().unwrap(), None);
    // No request must have been made — this is a pure cache read.
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn cached_policy_reflects_a_prior_fetch_without_any_new_request() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_json(sample_policy_json(7, 3600, "warn")),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache());
    policy_client.fetch().await.unwrap();

    // A second call to `cached_policy()` must not trigger a second request
    // (the mock's `.expect(1)` above would fail the test otherwise).
    let policy = policy_client.cached_policy().unwrap().unwrap();
    assert_eq!(policy.policy_version, 7);
    let _ = policy_client.cached_policy().unwrap().unwrap();
}

#[tokio::test]
async fn cached_policy_with_corrupt_cache_is_cache_corrupt() {
    let mock = MockServer::start().await;
    let backend = Arc::new(InMemoryBackend::with_bytes(
        br#"{"policy": "not-a-policy-object", "fetched_at_epoch_secs": 1}"#.to_vec(),
    ));
    let policy_client = PolicyClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        PolicyCache::new(backend),
        mock.uri(),
        "myapp",
    );
    let err = policy_client.cached_policy().unwrap_err();
    assert!(matches!(err, PolicyClientError::CacheCorrupt { .. }));
}

// ── refresh_managed_config (Bug 4) ──────────────────────────────────────────
//
// The centerpiece fix: connects `PolicyClient` to a real, typed
// `ConfigStore<T>` — the entire manifest/resolver/PolicyClient machinery
// above was previously never invoked by any path a real application actually
// uses.

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct RefreshAppConfig {
    schema_version: u32,
    greeting: String,
}

impl VersionedConfig for RefreshAppConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

fn refresh_manifest() -> ConfigManifest {
    ConfigManifest::new(
        "myapp",
        vec![FieldManifest {
            key: "greeting".to_string(),
            kind: FieldKind::Str,
            default: Some(json!("hello")),
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
        }],
    )
}

fn refresh_store() -> ConfigStore<RefreshAppConfig> {
    ConfigStore::new(Arc::new(InMemoryBackend::new()), ConfigFormat::default(), 1)
}

#[tokio::test]
async fn refresh_managed_config_applies_a_fresh_enforced_value_and_notifies_subscriber() {
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
            "enforced": { "greeting": "org-mandated" },
            "recommended": {},
        })))
        .mount(&mock)
        .await;

    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache());
    let store = refresh_store();
    store.resolve().unwrap();
    assert_eq!(store.current().greeting, "");

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_clone = seen.clone();
    store.subscribe(move |cfg| seen_clone.lock().unwrap().push(cfg.greeting.clone()));

    let manifest = refresh_manifest();
    let outcome = refresh_managed_config(&store, &manifest, &policy_client)
        .await
        .unwrap();

    match outcome {
        PolicyOutcome::Fresh(policy) => assert_eq!(policy.policy_version, 1),
        other => panic!("expected Fresh, got {other:?}"),
    }
    assert_eq!(
        store.current().greeting,
        "org-mandated",
        "store.current() must reflect the fetched enforced value"
    );
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["org-mandated".to_string()],
        "a subscriber registered before the call must fire with the new value"
    );
}

/// The negative case: a `Denied` outcome (401-after-retry or 403) must not
/// corrupt the running config either — `store.current()` must still reflect
/// exactly the same value as before the call (folding with empty
/// enforced/recommended trees, per spec 021, reproduces the identical
/// local-only value rather than clearing or defaulting it).
#[tokio::test]
async fn refresh_managed_config_on_denied_leaves_store_current_unchanged() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&mock)
        .await;

    let policy_client = client(&mock, Arc::new(FixedTokenProvider("tok")), cache());
    let store = refresh_store();
    store.resolve().unwrap();
    let mut seeded = (*store.current()).clone();
    seeded.greeting = "local-value".to_string();
    store.save(&seeded).unwrap();
    assert_eq!(store.current().greeting, "local-value");

    let manifest = refresh_manifest();
    let outcome = refresh_managed_config(&store, &manifest, &policy_client)
        .await
        .unwrap();

    assert_eq!(outcome, PolicyOutcome::Denied);
    assert_eq!(
        store.current().greeting,
        "local-value",
        "Denied must not corrupt the running config"
    );
}

// ── ConfigHandle::set_current_json_and_notify (Bug 4 primitive) ─────────────

/// The object-safe counterpart of `ConfigStore::set_current_and_notify`, used
/// by `config refresh`'s type-erased path (`apply_policy_outcome_to_handle`).
/// The happy path is already exercised end-to-end by
/// `tests/integration/config_commands.rs`'s
/// `config_refresh_actually_updates_the_live_store_not_just_prints_a_message`;
/// this covers its error path directly: a JSON document that doesn't
/// deserialize into `T` must surface `ConfigError::Parse`, not panic, and
/// must leave the store's current value untouched.
#[test]
fn set_current_json_and_notify_on_type_mismatch_is_parse_error_and_leaves_current_untouched() {
    let store = ConfigStore::<RefreshAppConfig>::new(
        Arc::new(InMemoryBackend::new()),
        ConfigFormat::default(),
        1,
    );
    store.resolve().unwrap();
    assert_eq!(store.current().greeting, "");

    let handle: &dyn ConfigHandle = &store;
    // `greeting` is a `String` on `RefreshAppConfig`; a JSON integer there
    // cannot deserialize into it.
    let err = handle
        .set_current_json_and_notify(json!({"schema_version": 1, "greeting": 42}))
        .unwrap_err();
    assert!(matches!(
        err,
        cli_framework::config::ConfigError::Parse { .. }
    ));
    assert_eq!(
        store.current().greeting,
        "",
        "a failed set_current_json_and_notify must not mutate the store's current value"
    );
}

// ── Policy JSON sanity (used above) ─────────────────────────────────────────

#[test]
fn sample_policy_json_parses_as_policy() {
    let value = sample_policy_json(1, 60, "refuse");
    let policy: Policy = serde_json::from_value(value).unwrap();
    assert_eq!(policy.stale_action, StaleAction::Refuse);
    assert!(policy.enforced.is_empty());
}

#[test]
fn empty_map_helper_compiles() {
    let _: Map<String, serde_json::Value> = Map::new();
}

// ── RoamingConfigClient ──────────────────────────────────────────────────────

fn leaf(key: &str, scope: Scope) -> FieldManifest {
    FieldManifest {
        key: key.to_string(),
        kind: FieldKind::Str,
        default: None,
        label: None,
        description: None,
        group: None,
        scope,
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

fn roaming_manifest() -> ConfigManifest {
    ConfigManifest::new(
        "myapp",
        vec![
            leaf("nickname", Scope::User),
            leaf("theme", Scope::User),
            leaf("install_id", Scope::Machine),
        ],
    )
}

#[tokio::test]
async fn roaming_get_reads_document_and_etag() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/config/myapp"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"doc-v1\"")
                .set_body_json(json!({"nickname": "alice"})),
        )
        .mount(&mock)
        .await;

    let roaming = RoamingConfigClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        mock.uri(),
        "myapp",
    );
    let doc = roaming.get().await.unwrap();
    assert_eq!(doc.value.get("nickname"), Some(&json!("alice")));
    assert_eq!(doc.etag.as_deref(), Some("\"doc-v1\""));
}

#[tokio::test]
async fn roaming_put_sends_only_user_scoped_fields() {
    let mock = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/v1/config/myapp"))
        .and(header("if-match", "\"doc-v1\""))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    let roaming = RoamingConfigClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        mock.uri(),
        "myapp",
    );
    let mut doc = Map::new();
    doc.insert("nickname".to_string(), json!("alice"));
    doc.insert("install_id".to_string(), json!("machine-123")); // must be dropped

    roaming
        .put(&roaming_manifest(), &doc, "\"doc-v1\"")
        .await
        .unwrap();

    // Verify the actual request body wiremock received contained only the
    // user-scoped field — a positive check on the wire body, not just on
    // `filter_user_scoped` in isolation.
    let requests = mock.received_requests().await.unwrap();
    let put_req = requests
        .iter()
        .find(|r| r.method.as_str() == "PUT")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&put_req.body).unwrap();
    assert_eq!(body, json!({"nickname": "alice"}));
}

#[tokio::test]
async fn roaming_put_rejects_a_conflicting_if_match() {
    let mock = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/v1/config/myapp"))
        .respond_with(ResponseTemplate::new(412))
        .mount(&mock)
        .await;

    let roaming = RoamingConfigClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        mock.uri(),
        "myapp",
    );
    let mut doc = Map::new();
    doc.insert("nickname".to_string(), json!("bob"));

    let err = roaming
        .put(&roaming_manifest(), &doc, "\"stale-etag\"")
        .await
        .unwrap_err();
    assert!(matches!(err, RoamingClientError::Conflict));
}

#[tokio::test]
async fn roaming_get_with_empty_body_returns_an_empty_document() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/config/myapp"))
        .respond_with(ResponseTemplate::new(200)) // no body at all
        .mount(&mock)
        .await;

    let roaming = RoamingConfigClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        mock.uri(),
        "myapp",
    );
    let doc = roaming.get().await.unwrap();
    assert!(doc.value.is_empty());
}

#[tokio::test]
async fn roaming_put_unexpected_status_is_a_request_error() {
    let mock = MockServer::start().await;
    // 201 is a 2xx (so the underlying retry client treats it as `Ok`), but
    // `put()` itself only special-cases 200/204 — exercises its own
    // `other => ...` arm rather than the shared retry/error-classification
    // machinery.
    Mock::given(method("PUT"))
        .and(path("/v1/config/myapp"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&mock)
        .await;

    let roaming = RoamingConfigClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        mock.uri(),
        "myapp",
    );
    let mut doc = Map::new();
    doc.insert("nickname".to_string(), json!("bob"));
    let err = roaming
        .put(&roaming_manifest(), &doc, "\"etag\"")
        .await
        .unwrap_err();
    assert!(matches!(err, RoamingClientError::Request(_)));
}

#[tokio::test]
async fn roaming_put_non_conflict_failure_is_a_request_error_not_a_conflict() {
    let mock = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/v1/config/myapp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let roaming = RoamingConfigClient::new(
        authed_client(Arc::new(FixedTokenProvider("tok"))),
        mock.uri(),
        "myapp",
    );
    let mut doc = Map::new();
    doc.insert("nickname".to_string(), json!("bob"));
    let err = roaming
        .put(&roaming_manifest(), &doc, "\"etag\"")
        .await
        .unwrap_err();
    assert!(
        matches!(err, RoamingClientError::Request(_)),
        "a plain 500 must not be misclassified as Conflict; got {err:?}"
    );
}
