//! The managed-configuration client (spec 021, "the managed client"):
//! [`PolicyClient`] fetches and caches an organisation's [`crate::config::Policy`],
//! and [`RoamingConfigClient`] reads/writes the user-scoped roaming document.
//!
//! Gated behind the `config-managed` feature (implying `config` + `auth`) —
//! the [`crate::config::Policy`] document type itself, and the resolver in
//! [`crate::config::resolution`], have no networking dependency and remain
//! available under plain `config`. Only the HTTP fetchers live here.
//!
//! Built on top of [`crate::auth::AuthenticatedHttpClient`] rather than
//! reimplementing bearer-token injection or 401 retry: the "attempt token
//! acquisition once via TokenProvider" step spec 021's failure-mapping table
//! calls for is exactly `AuthenticatedHttpClient`'s existing invalidate +
//! retry-once behavior (see `crate::auth`).

mod cache;
mod policy_client;
mod roaming_client;

pub use cache::{now_epoch_secs, PolicyCache, PolicyCacheEntry};
pub use policy_client::{PolicyClient, PolicyClientError, PolicyOutcome};
pub use roaming_client::{
    filter_user_scoped, RoamingClientError, RoamingConfigClient, RoamingDocument,
};

use crate::config::manifest::ConfigManifest;
use crate::config::resolution::{flatten_to_paths, resolve, unflatten_from_paths, ResolutionInput};
use crate::config::{ConfigHandle, ConfigStore, VersionedConfig};
use serde_json::{Map, Value};

// ── Connecting `PolicyClient` to a typed `ConfigStore<T>` ───────────────────
//
// This is what actually wires the manifest/resolver/PolicyClient machinery
// above to the value an application's own typed config code sees (spec 021's
// headline "enforced beats everything" promise) — `AppBuilder::build_with_config`
// alone never touched the network or the resolver before this; see
// `fold_cached_policy_into_value` (the synchronous, cache-only half `build()`
// uses) and `refresh_managed_config` (the explicit async half an application
// calls itself, per spec 021's "Refresh": "an application may opt into a
// change callback... built on PRD 016's reload seam").

/// Extract the `(enforced, recommended)` trees a [`PolicyOutcome`] carries, if
/// any. [`PolicyOutcome::Unmanaged`] and [`PolicyOutcome::Denied`] carry no
/// [`crate::config::Policy`] at all — folding resolves with both empty, i.e.
/// falls back to local-only, exactly as an application with no managed
/// configuration at all would.
fn policy_trees(outcome: &PolicyOutcome) -> (Map<String, Value>, Map<String, Value>) {
    match outcome {
        PolicyOutcome::Fresh(policy) | PolicyOutcome::FromCache { policy, .. } => {
            (policy.enforced.clone(), policy.recommended.clone())
        }
        PolicyOutcome::Unmanaged | PolicyOutcome::Denied => (Map::new(), Map::new()),
    }
}

/// The core fold shared by every entry point below: flatten `current_value_json`
/// (the local/already-resolved value, becoming the `config_file` layer),
/// run [`resolve`] with the given `enforced`/`recommended` trees, then
/// [`unflatten_from_paths`] the result back into the nested shape
/// `serde_json::from_value::<T>` needs. Infallible — `resolve`/`unflatten`
/// never fail; only serializing in or deserializing out at the call sites
/// below can.
///
/// The manifest-shaped result is **merged onto a clone of
/// `current_value_json`**, not returned as-is: a manifest only ever declares
/// an application's *configurable* fields, so a `VersionedConfig`'s own
/// bookkeeping (`schema_version`) — or any other field the manifest simply
/// doesn't mention — would otherwise be silently dropped by
/// [`unflatten_from_paths`], since it only ever reconstructs paths it knows
/// about. [`merge_json_objects`] preserves everything not covered by the
/// manifest while still letting every manifest-declared field (recursively,
/// for nested sections) be overwritten by its freshly resolved value.
fn fold_into_json(
    manifest: &ConfigManifest,
    current_value_json: &Value,
    enforced: Map<String, Value>,
    recommended: Map<String, Value>,
) -> Value {
    let config_file = flatten_to_paths(current_value_json);
    let input = ResolutionInput {
        recommended,
        enforced,
        config_file,
        ..Default::default()
    };
    let resolved = resolve(manifest, &input);
    let mut flat = Map::new();
    for entry in resolved.entries() {
        flat.insert(entry.path, entry.value);
    }
    let manifest_shaped = unflatten_from_paths(manifest, &flat);
    let mut merged = current_value_json.clone();
    merge_json_objects(&mut merged, &manifest_shaped);
    merged
}

/// Recursively overlay `overlay`'s object keys onto `base`: a leaf value is
/// replaced outright, but two nested objects at the same key are merged
/// (recursively) rather than one wholesale-replacing the other. See
/// [`fold_into_json`] for why this matters — it is what lets a
/// manifest-declared field's freshly resolved value land in the right place
/// while every field the manifest never mentions survives untouched.
fn merge_json_objects(base: &mut Value, overlay: &Value) {
    if let (Value::Object(base_map), Value::Object(overlay_map)) = (&mut *base, overlay) {
        for (key, overlay_value) in overlay_map {
            match base_map.get_mut(key) {
                Some(base_value) if base_value.is_object() && overlay_value.is_object() => {
                    merge_json_objects(base_value, overlay_value);
                }
                _ => {
                    base_map.insert(key.clone(), overlay_value.clone());
                }
            }
        }
    }
}

/// The synchronous, **local-cache-only** half of connecting a `PolicyClient`
/// to a typed `ConfigStore<T>` — used by
/// [`crate::app::AppBuilder::build_with_config`]. Performs **no network
/// request**: only [`PolicyClient::cached_policy`] (a plain file read) is
/// consulted, so a one-shot CLI's synchronous `build()` path never blocks on,
/// or silently depends on, the managed-config server being reachable. The
/// network-fetching counterpart applications call explicitly is
/// [`refresh_managed_config`].
pub(crate) fn fold_cached_policy_into_value<T: VersionedConfig>(
    store: &ConfigStore<T>,
    manifest: &ConfigManifest,
    policy_client: &PolicyClient,
) -> Result<T, PolicyClientError> {
    let cached = policy_client.cached_policy()?;
    let (enforced, recommended) = match cached {
        Some(policy) => (policy.enforced, policy.recommended),
        None => (Map::new(), Map::new()),
    };
    // `load()`, not `store.current()`: the `config_file` layer must always be
    // what the backend actually says, never a previously policy-folded
    // `current` value — see `ConfigHandle::backend_json`'s docs for why
    // reusing `current` here would let a withdrawn policy's value survive
    // indefinitely, mistaken for the user's own local setting.
    let local: T = store.load().map_err(|e| PolicyClientError::FoldFailed {
        app: manifest.app.clone(),
        message: format!("local config could not be read from the backend: {e}"),
    })?;
    let current_json = serde_json::to_value(&local).map_err(|e| PolicyClientError::FoldFailed {
        app: manifest.app.clone(),
        message: format!("local config value could not be serialized: {e}"),
    })?;
    let nested = fold_into_json(manifest, &current_json, enforced, recommended);
    serde_json::from_value(nested).map_err(|e| PolicyClientError::FoldFailed {
        app: manifest.app.clone(),
        message: format!("policy-folded value did not match the config type: {e}"),
    })
}

/// Fetch the current [`crate::config::Policy`] and fold it into `store`'s
/// resolved value via `manifest`, updating the store's live cached value
/// (never the backend file — see [`ConfigStore::set_current_and_notify`])
/// and notifying subscribers registered via [`ConfigStore::subscribe`].
///
/// This is what actually connects [`PolicyClient`] to a typed
/// `ConfigStore<T>` for a running application — call it at startup (awaited)
/// and/or on whatever interval [`PolicyClient::background_refresh`] already
/// exists for. [`AppBuilder::build_with_config`][crate::app::AppBuilder::build_with_config]
/// deliberately does **not** call this itself (it is synchronous and must
/// never touch the network — see [`fold_cached_policy_into_value`]); this is
/// the explicit async step spec 021's "Refresh" describes as something an
/// application orchestrates, not something a synchronous builder call
/// performs silently.
///
/// Returns the [`PolicyOutcome`] so the caller (e.g. the `config refresh`
/// command, or an application's own startup code) can still inspect what
/// happened (stale? denied? unmanaged?) exactly as
/// [`PolicyClient::fetch`] itself reports.
///
/// **Concurrency bound, stated plainly.** The network fetch above is the only
/// `.await` point; everything from the backend read through the final
/// [`ConfigStore::set_current_and_notify`] call is synchronous, and that call
/// commits under the same `write_lock` [`ConfigStore::save`] uses, so the two
/// cannot interleave into a torn `current` value — whichever commits last
/// wins, exactly the guarantee `save` already gives two concurrent savers.
/// What this does **not** provide is a transaction across the whole refresh:
/// a `save` landing on another thread strictly between this call's backend
/// read and its commit can still have its *live* effect transiently
/// overwritten (its write to the backend file is never at risk — only the
/// in-memory `current` view is). That view self-corrects on the next
/// `save`/`reload`/`refresh_managed_config` call. Closing this fully would
/// mean holding `write_lock` across the network fetch above, which was
/// rejected: it would block a user's settings save for the duration of a
/// policy server round-trip.
pub async fn refresh_managed_config<T: VersionedConfig>(
    store: &ConfigStore<T>,
    manifest: &ConfigManifest,
    policy_client: &PolicyClient,
) -> Result<PolicyOutcome, PolicyClientError> {
    let outcome = policy_client.fetch().await?;
    let (enforced, recommended) = policy_trees(&outcome);
    // Same reasoning as `fold_cached_policy_into_value`: read the backend
    // fresh, not `store.current()`, so a field a *previous* refresh applied
    // from Policy never gets laundered back in as if it were locally
    // authored once the current policy no longer mentions it.
    let local: T = store.load().map_err(|e| PolicyClientError::FoldFailed {
        app: manifest.app.clone(),
        message: format!("local config could not be read from the backend: {e}"),
    })?;
    let current_json = serde_json::to_value(&local).map_err(|e| PolicyClientError::FoldFailed {
        app: manifest.app.clone(),
        message: format!("local config value could not be serialized: {e}"),
    })?;
    let nested = fold_into_json(manifest, &current_json, enforced, recommended);
    let value: T = serde_json::from_value(nested).map_err(|e| PolicyClientError::FoldFailed {
        app: manifest.app.clone(),
        message: format!("policy-folded value did not match the config type: {e}"),
    })?;
    store.set_current_and_notify(value);
    Ok(outcome)
}

/// Type-erased counterpart of folding an already-fetched [`PolicyOutcome`]
/// into a live config value, operating purely through the object-safe
/// [`ConfigHandle`] — used by the built-in `config refresh` command
/// (`crate::config::commands`), which only ever sees `&dyn ConfigHandle`
/// (its `T` is erased) and therefore cannot call the generic
/// [`refresh_managed_config`] directly. Deliberately synchronous: the
/// network fetch already happened by the time this runs, so there is no
/// `&mut dyn AppContext` borrow to hold across an `.await` here.
pub(crate) fn apply_policy_outcome_to_handle(
    handle: &dyn ConfigHandle,
    manifest: &ConfigManifest,
    outcome: &PolicyOutcome,
) -> Result<(), PolicyClientError> {
    let (enforced, recommended) = policy_trees(outcome);
    // `backend_json()`, not `current_json()` — see `ConfigHandle::backend_json`'s
    // docs and the identical reasoning in `fold_cached_policy_into_value`.
    let current_json = handle
        .backend_json()
        .map_err(|e| PolicyClientError::FoldFailed {
            app: manifest.app.clone(),
            message: format!("local config could not be read from the backend: {e}"),
        })?;
    let nested = fold_into_json(manifest, &current_json, enforced, recommended);
    handle
        .set_current_json_and_notify(nested)
        .map_err(|e| PolicyClientError::FoldFailed {
            app: manifest.app.clone(),
            message: format!("policy-folded value could not be applied to the running config: {e}"),
        })
}

// ── Internal fold-path coverage ──────────────────────────────────────────────
//
// `fold_cached_policy_into_value`, `apply_policy_outcome_to_handle`, and the
// private helpers above are `pub(crate)`/private, not part of the public
// surface `tests/unit/config_managed_client.rs` exercises — some of their
// edge cases (a serialization failure, a manifest/struct type mismatch after
// folding) also aren't reachable through the public `AppBuilder`/`PolicyClient`
// API at all (e.g. `AppBuilder::build_with_config` always resolves `T` fresh
// from the backend immediately before folding, so `store.current()` is never
// in an "unrepresentable" state by the time its own fold runs). Matches the
// inline-test convention already used by `cache.rs` in this same module for
// exactly this reason.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{FieldKind, FieldManifest, Scope};
    use crate::config::{ConfigFormat, InMemoryBackend};
    use serde_json::json;
    use std::sync::Arc;

    fn empty_manifest() -> ConfigManifest {
        ConfigManifest::new("app", vec![])
    }

    fn unreachable_policy_client() -> PolicyClient {
        PolicyClient::new(
            Arc::new(crate::auth::AuthenticatedHttpClient::new(
                crate::http_retry::RetryableHttpClient::new(reqwest::Client::new()),
                Arc::new(NeverCalledTokenProvider) as Arc<dyn crate::auth::TokenProvider>,
            )),
            PolicyCache::new(Arc::new(InMemoryBackend::new())),
            "http://127.0.0.1:1",
            "app",
        )
    }

    /// Never actually invoked by any test here — `cached_policy()` is a pure
    /// file read with no network access, so this provider only needs to
    /// exist to satisfy `AuthenticatedHttpClient::new`'s constructor.
    struct NeverCalledTokenProvider;
    #[async_trait::async_trait]
    impl crate::auth::TokenProvider for NeverCalledTokenProvider {
        async fn token(&self) -> Result<crate::auth::AccessToken, crate::auth::AuthError> {
            Err(crate::auth::AuthError::NotAuthenticated)
        }
        async fn invalidate(&self) {}
    }

    /// Unlike [`NeverCalledTokenProvider`], this one IS actually invoked —
    /// for a test that calls [`PolicyClient::fetch`] against an unreachable
    /// server and needs the failure to be a genuine network/connect error
    /// (`HttpFailureClass::ServerOrNetwork`, which falls back to cache), not
    /// a token-acquisition failure (`HttpFailureClass::Unauthorized`, which
    /// would short-circuit straight to `PolicyOutcome::Denied` before any
    /// HTTP request is even attempted — see
    /// `unauthorized_where_retry_also_fails_does_not_read_cache`'s sibling
    /// test in `tests/unit/config_managed_client.rs` for that exact trap).
    struct AlwaysOkTokenProvider;
    #[async_trait::async_trait]
    impl crate::auth::TokenProvider for AlwaysOkTokenProvider {
        async fn token(&self) -> Result<crate::auth::AccessToken, crate::auth::AuthError> {
            Ok(crate::auth::AccessToken::new("tok".to_string(), None))
        }
        async fn invalidate(&self) {}
    }

    // ── merge_json_objects / fold_into_json ─────────────────────────────────

    #[test]
    fn merge_json_objects_recurses_into_nested_objects_at_the_same_key() {
        let mut base = json!({
            "network": { "proxy_url": "old", "port": 80 },
            "untouched": "keep-me",
        });
        let overlay = json!({
            "network": { "proxy_url": "new" },
        });
        merge_json_objects(&mut base, &overlay);
        assert_eq!(
            base,
            json!({
                "network": { "proxy_url": "new", "port": 80 },
                "untouched": "keep-me",
            })
        );
    }

    #[test]
    fn merge_json_objects_replaces_a_leaf_outright_not_recursing_into_it() {
        let mut base = json!({"count": 1});
        let overlay = json!({"count": 2});
        merge_json_objects(&mut base, &overlay);
        assert_eq!(base, json!({"count": 2}));
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

    #[test]
    fn fold_into_json_preserves_fields_the_manifest_never_declares() {
        // `schema_version` isn't a manifest field (it's `VersionedConfig`
        // bookkeeping) — it must survive the fold untouched, exactly the bug
        // this test pins (see `fold_into_json`'s own docs).
        let manifest =
            ConfigManifest::new("app", vec![field("greeting", FieldKind::Str, json!(""))]);
        let current = json!({"schema_version": 3, "greeting": "local"});
        let mut enforced = Map::new();
        enforced.insert("greeting".to_string(), json!("org-mandated"));
        let result = fold_into_json(&manifest, &current, enforced, Map::new());
        assert_eq!(
            result,
            json!({"schema_version": 3, "greeting": "org-mandated"})
        );
    }

    #[test]
    fn fold_into_json_recurses_through_a_nested_section() {
        let manifest = ConfigManifest::new(
            "app",
            vec![FieldManifest {
                kind: FieldKind::Section {
                    fields: vec![field("proxy_url", FieldKind::Str, json!(""))],
                },
                ..field("network", FieldKind::Bool, json!(false))
            }],
        );
        let current = json!({"schema_version": 1, "network": {"proxy_url": "old"}});
        let mut enforced = Map::new();
        enforced.insert("network.proxy_url".to_string(), json!("org-proxy"));
        let result = fold_into_json(&manifest, &current, enforced, Map::new());
        assert_eq!(
            result,
            json!({"schema_version": 1, "network": {"proxy_url": "org-proxy"}})
        );
    }

    // ── fold_cached_policy_into_value ────────────────────────────────────────

    #[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
    struct SimpleConfig {
        schema_version: u32,
        greeting: String,
    }
    impl VersionedConfig for SimpleConfig {
        fn schema_version(&self) -> u32 {
            self.schema_version
        }
        fn set_schema_version(&mut self, v: u32) {
            self.schema_version = v;
        }
    }

    // Regression: a value a *prior* fold pushed into `current` via
    // `set_current_and_notify` (never touching the backend) must not survive
    // into a *later* fold once the policy that produced it is gone — the
    // `config_file` layer must always come from the real backend, never from
    // `current`. Before the fix, this failed by returning "org-mandated"
    // (the stale value) instead of "local-value" (what the backend, and
    // therefore a genuinely local setting, actually says).
    #[test]
    fn fold_does_not_launder_a_prior_policy_fold_back_in_as_local_authorship() {
        let store = ConfigStore::<SimpleConfig>::new(
            Arc::new(InMemoryBackend::new()),
            ConfigFormat::default(),
            1,
        );
        store.resolve().unwrap();
        store
            .save(&SimpleConfig {
                schema_version: 1,
                greeting: "local-value".to_string(),
            })
            .unwrap();

        // Simulate a prior refresh that enforced "org-mandated" — this
        // mutates `current` only, exactly like `refresh_managed_config`
        // does; the backend still says "local-value".
        store.set_current_and_notify(SimpleConfig {
            schema_version: 1,
            greeting: "org-mandated".to_string(),
        });
        assert_eq!(store.current().greeting, "org-mandated");

        // The policy is withdrawn: nothing cached this time.
        let manifest =
            ConfigManifest::new("app", vec![field("greeting", FieldKind::Str, json!("hi"))]);
        let value =
            fold_cached_policy_into_value(&store, &manifest, &unreachable_policy_client()).unwrap();
        assert_eq!(
            value.greeting, "local-value",
            "must read the real backend value, not the stale in-memory `current` \
             a withdrawn policy previously wrote"
        );
    }

    #[test]
    fn fold_cached_policy_into_value_with_nothing_cached_falls_back_to_local_only() {
        let store = ConfigStore::<SimpleConfig>::new(
            Arc::new(InMemoryBackend::new()),
            ConfigFormat::default(),
            1,
        );
        store.resolve().unwrap();
        // Seed a local value distinct from the manifest default, so the
        // assertion below actually distinguishes "config_file (local) wins,
        // as always, since there's no cache to contribute recommended/
        // enforced at all" from "the manifest default happened to apply."
        store
            .save(&SimpleConfig {
                schema_version: 1,
                greeting: "local-value".to_string(),
            })
            .unwrap();

        let manifest =
            ConfigManifest::new("app", vec![field("greeting", FieldKind::Str, json!("hi"))]);
        let value =
            fold_cached_policy_into_value(&store, &manifest, &unreachable_policy_client()).unwrap();
        assert_eq!(
            value.greeting, "local-value",
            "no cache -> falls back to local-only, i.e. the existing config_file value survives \
             untouched rather than being overridden by the manifest default"
        );
    }

    /// A store whose *backend* holds bytes that don't parse as JSON at all —
    /// what `fold_cached_policy_into_value`/`refresh_managed_config`/
    /// `apply_policy_outcome_to_handle` now read via `store.load()`/
    /// `handle.backend_json()` (Bug 6's fix: the fold path was moved off
    /// `store.current()` onto a fresh backend read, specifically so a stale
    /// *in-memory* value can never be mistaken for local authorship — see
    /// those functions' docs). Corrupting the backend directly, rather than
    /// poisoning `current` via `set_current_and_notify` (the original
    /// technique here, before that fix), is what actually exercises the
    /// current read path: `store.load()` returns `T::default()` on an empty
    /// backend without ever touching serde_json, so a `current`-only defect
    /// no longer reaches it at all — confirmed by these three tests failing
    /// (wrongly returning `Ok`) against `unrepresentable_store()`'s original
    /// form immediately after Bug 6 landed.
    fn store_with_corrupt_backend() -> ConfigStore<SimpleConfig> {
        let store = ConfigStore::<SimpleConfig>::new(
            Arc::new(InMemoryBackend::with_bytes(b"not json at all".to_vec())),
            ConfigFormat::default(),
            1,
        );
        // Seed `current` with something well-formed, so a test relying on
        // this fixture is unambiguously exercising the *backend* read this
        // fixture exists to corrupt, not an unrelated `current`-side defect.
        store.set_current_and_notify(SimpleConfig {
            schema_version: 1,
            greeting: "current-is-fine".to_string(),
        });
        store
    }

    #[test]
    fn fold_cached_policy_into_value_surfaces_a_backend_read_failure() {
        let store = store_with_corrupt_backend();
        let err =
            fold_cached_policy_into_value(&store, &empty_manifest(), &unreachable_policy_client())
                .unwrap_err();
        assert!(matches!(err, PolicyClientError::FoldFailed { .. }));
    }

    #[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
    struct TypeMismatchConfig {
        schema_version: u32,
        flag: bool,
    }
    impl VersionedConfig for TypeMismatchConfig {
        fn schema_version(&self) -> u32 {
            self.schema_version
        }
        fn set_schema_version(&mut self, v: u32) {
            self.schema_version = v;
        }
    }

    /// The manifest declares `flag` as an integer (a value the resolver's own
    /// `value_matches_kind` check happily accepts), but the real Rust struct
    /// field is `bool` — a manifest/struct drift the resolver cannot see,
    /// only surfacing once the folded JSON is deserialized back into `T`.
    fn type_mismatch_manifest() -> ConfigManifest {
        ConfigManifest::new("app", vec![field("flag", FieldKind::Int, json!(0))])
    }

    #[test]
    fn fold_cached_policy_into_value_surfaces_a_manifest_struct_type_mismatch() {
        let store = ConfigStore::<TypeMismatchConfig>::new(
            Arc::new(InMemoryBackend::new()),
            ConfigFormat::default(),
            1,
        );
        store.resolve().unwrap();

        let policy_cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
        let mut enforced = Map::new();
        enforced.insert("flag".to_string(), json!(42));
        policy_cache
            .write(&PolicyCacheEntry {
                policy: json!({
                    "contract_version": 1,
                    "app": "app",
                    "profile": "developers",
                    "policy_version": 1,
                    "max_cache_age_secs": 3600,
                    "stale_action": "warn",
                    "enforced": enforced,
                    "recommended": {},
                }),
                etag: None,
                fetched_at_epoch_secs: now_epoch_secs(),
            })
            .unwrap();
        let policy_client = PolicyClient::new(
            Arc::new(crate::auth::AuthenticatedHttpClient::new(
                crate::http_retry::RetryableHttpClient::new(reqwest::Client::new()),
                Arc::new(NeverCalledTokenProvider) as Arc<dyn crate::auth::TokenProvider>,
            )),
            policy_cache,
            "http://127.0.0.1:1",
            "app",
        );

        let err = fold_cached_policy_into_value(&store, &type_mismatch_manifest(), &policy_client)
            .unwrap_err();
        assert!(matches!(err, PolicyClientError::FoldFailed { .. }));
    }

    // ── apply_policy_outcome_to_handle ───────────────────────────────────────

    #[test]
    fn apply_policy_outcome_to_handle_surfaces_a_backend_read_failure() {
        let store = store_with_corrupt_backend();
        let handle: &dyn ConfigHandle = &store;
        let err =
            apply_policy_outcome_to_handle(handle, &empty_manifest(), &PolicyOutcome::Unmanaged)
                .unwrap_err();
        assert!(matches!(err, PolicyClientError::FoldFailed { .. }));
    }

    #[test]
    fn apply_policy_outcome_to_handle_surfaces_a_manifest_struct_type_mismatch() {
        let store = ConfigStore::<TypeMismatchConfig>::new(
            Arc::new(InMemoryBackend::new()),
            ConfigFormat::default(),
            1,
        );
        store.resolve().unwrap();
        let handle: &dyn ConfigHandle = &store;

        let mut enforced = Map::new();
        enforced.insert("flag".to_string(), json!(42));
        let outcome = PolicyOutcome::Fresh(crate::config::Policy {
            contract_version: 1,
            app: "app".to_string(),
            profile: "developers".to_string(),
            policy_version: 1,
            max_cache_age_secs: 3600,
            stale_action: crate::config::StaleAction::Warn,
            enforced,
            recommended: Map::new(),
        });

        let err = apply_policy_outcome_to_handle(handle, &type_mismatch_manifest(), &outcome)
            .unwrap_err();
        assert!(matches!(err, PolicyClientError::FoldFailed { .. }));
    }

    // ── refresh_managed_config (serialization / type-mismatch edges) ────────
    //
    // The success path (a fresh enforced value applied and a subscriber
    // notified) and the `Denied` negative case are covered in
    // `tests/unit/config_managed_client.rs` against the public API; these two
    // cover the edges only reachable via the internal fixtures above.

    #[tokio::test]
    async fn refresh_managed_config_surfaces_a_backend_read_failure() {
        let store = store_with_corrupt_backend();
        let err = refresh_managed_config(&store, &empty_manifest(), &unreachable_policy_client())
            .await
            .unwrap_err();
        assert!(matches!(err, PolicyClientError::FoldFailed { .. }));
    }

    #[tokio::test]
    async fn refresh_managed_config_surfaces_a_manifest_struct_type_mismatch() {
        let store = ConfigStore::<TypeMismatchConfig>::new(
            Arc::new(InMemoryBackend::new()),
            ConfigFormat::default(),
            1,
        );
        store.resolve().unwrap();

        let mut enforced = Map::new();
        enforced.insert("flag".to_string(), json!(42));
        let policy_cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
        policy_cache
            .write(&PolicyCacheEntry {
                policy: json!({
                    "contract_version": 1,
                    "app": "app",
                    "profile": "developers",
                    "policy_version": 1,
                    "max_cache_age_secs": 3600,
                    "stale_action": "warn",
                    "enforced": enforced,
                    "recommended": {},
                }),
                etag: None,
                fetched_at_epoch_secs: now_epoch_secs(),
            })
            .unwrap();
        // Server unreachable -> falls back to the cache above (fresh enough,
        // default max_cache_age_secs), carrying the same type-mismatched
        // `flag`. Exercises `refresh_managed_config`'s fold failure via the
        // `FromCache` branch of `policy_trees` rather than `Fresh`. Needs a
        // token provider that actually succeeds (see
        // `AlwaysOkTokenProvider`'s docs) so the failure is a genuine
        // network error, not a token-acquisition one.
        let policy_client = PolicyClient::new(
            Arc::new(crate::auth::AuthenticatedHttpClient::new(
                crate::http_retry::RetryableHttpClient::new(reqwest::Client::new()),
                Arc::new(AlwaysOkTokenProvider) as Arc<dyn crate::auth::TokenProvider>,
            )),
            policy_cache,
            "http://127.0.0.1:1",
            "app",
        );

        let err = refresh_managed_config(&store, &type_mismatch_manifest(), &policy_client)
            .await
            .unwrap_err();
        assert!(matches!(err, PolicyClientError::FoldFailed { .. }));
    }
}
