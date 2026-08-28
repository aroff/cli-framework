//! [`ConfigServiceState`]: the shared state behind every handler in
//! [`super::router::config_service_router`] — the storage seam, the caller
//! identity seam, and the in-memory resolved-policy cache (spec 022,
//! "Caching": "Resolved policies are cached in memory keyed by application,
//! profile, and version, invalidated on version change, so a fleet
//! refreshing on interval does not translate into a database read per
//! device").

use super::assignment::resolve_profile;
use super::error::InheritanceError;
use super::identity::CallerIdentity;
use super::inherit::{combined_chain_version, flatten, resolve_chain};
use super::store::{PolicyStore, UserConfigStore};
use super::types::StoredPolicy;
use crate::config::Policy;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Default cap on a roaming user document's serialized size (spec 022 user
/// story 23: "user documents size-limited, so that the settings store
/// cannot be used as general-purpose storage"). 64 KiB is generous for a
/// settings document and small enough that the limit is meaningfully
/// enforcing something — a judgment call, since spec 022 names the
/// requirement but not a number; override with
/// [`ConfigServiceState::with_max_user_config_bytes`].
pub const DEFAULT_MAX_USER_CONFIG_BYTES: usize = 64 * 1024;

/// Why [`ConfigServiceState::lookup_policy`] / [`ConfigServiceState::resolve_diagnostic`]
/// could not produce a result. [`Self::Unmanaged`] is not a failure in the
/// ordinary sense — it is spec 022's "not-found means unmanaged" outcome —
/// but is modelled as an error variant here because it is, precisely, the
/// one case in which there is no value to return.
#[derive(Debug)]
pub enum PolicyLookupError {
    /// No assignment rule matched (and no default profile), or the resolved
    /// profile has no stored policy at all. The router maps this to `404`.
    Unmanaged,
    /// A storage error, or an inheritance-integrity failure that startup
    /// validation should have already caught (see
    /// [`super::validate::validate_all`]) — the router maps this to `500`
    /// and logs the detail rather than leaking it to the caller.
    Internal(String),
}

/// The `/v1/resolve/{app}` diagnostic body: profile and matching rule only,
/// no configuration values (spec 022 user story 12).
#[derive(Debug, Clone, Serialize)]
pub struct ResolveDiagnostic {
    pub profile: String,
    pub matched_rule: MatchedRule,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchedRule {
    pub ord: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_path: Option<String>,
    pub operator: super::types::RuleOperator,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// Shared state for every handler the config-service router registers.
pub struct ConfigServiceState {
    pub policy_store: Arc<dyn PolicyStore>,
    pub user_config_store: Arc<dyn UserConfigStore>,
    pub identity: Arc<dyn CallerIdentity>,
    pub max_user_config_bytes: usize,
    /// Keyed by `(app, profile, combined_chain_version)` — the version
    /// component is [`combined_chain_version`] over the *entire* resolved
    /// inheritance chain, not the leaf profile's own stored version alone.
    /// A served `Policy` is the flattened result of every profile in the
    /// chain, so the cache key (and the served ETag, which reuses the same
    /// value as `Policy::policy_version`) must change whenever **any**
    /// ancestor's stored version changes — keying on the leaf alone would
    /// let a parent-only edit hide behind a stale cache entry and a stale
    /// ETag indefinitely. A version change (anywhere in the chain) is a new
    /// key, so nothing needs active invalidation; a stale entry simply stops
    /// being looked up once its chain resolves to a newer combined version.
    /// Never evicted in this slice: each entry is small (one flattened
    /// `Policy`) and the key space is bounded by the number of distinct
    /// (app, profile, combined version) triples ever served, which is a
    /// deliberate, documented simplification rather than an oversight — see
    /// this slice's report for the trade-off.
    cache: Mutex<HashMap<(String, String, u64), Policy>>,
}

impl ConfigServiceState {
    pub fn new(
        policy_store: Arc<dyn PolicyStore>,
        user_config_store: Arc<dyn UserConfigStore>,
        identity: Arc<dyn CallerIdentity>,
    ) -> Arc<Self> {
        Arc::new(Self {
            policy_store,
            user_config_store,
            identity,
            max_user_config_bytes: DEFAULT_MAX_USER_CONFIG_BYTES,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Override the roaming-document size cap (see
    /// [`DEFAULT_MAX_USER_CONFIG_BYTES`]). Consumes and returns `self`
    /// wrapped in a fresh `Arc`, since [`Self::new`] already returns one —
    /// call this before handing the state to
    /// [`super::router::config_service_router`].
    pub fn with_max_user_config_bytes(self: Arc<Self>, max_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            policy_store: self.policy_store.clone(),
            user_config_store: self.user_config_store.clone(),
            identity: self.identity.clone(),
            max_user_config_bytes: max_bytes,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Validate every stored policy against its manifest — see
    /// [`super::validate::validate_all`]. Call this once at startup, before
    /// mounting the router; a non-empty error means the service must not
    /// start serving.
    pub async fn validate_at_startup(&self) -> Result<(), super::error::StartupValidationError> {
        super::validate::validate_all(self.policy_store.as_ref()).await
    }

    /// Resolve `claims` to a profile and, if one is found, its flattened,
    /// wire-ready [`Policy`] — reusing the in-memory cache when the
    /// resolved policy's version hasn't changed.
    pub async fn lookup_policy(
        &self,
        app: &str,
        claims: &Value,
    ) -> Result<Policy, PolicyLookupError> {
        let rules = self
            .policy_store
            .assignment_rules(app)
            .await
            .map_err(|e| PolicyLookupError::Internal(e.to_string()))?;
        let Some(resolved) = resolve_profile(&rules, claims) else {
            return Err(PolicyLookupError::Unmanaged);
        };
        let profile = resolved.profile().to_string();

        let all_policies = self
            .policy_store
            .policies_for_app(app)
            .await
            .map_err(|e| PolicyLookupError::Internal(e.to_string()))?;
        let by_profile: HashMap<&str, &StoredPolicy> = all_policies
            .iter()
            .map(|p| (p.profile.as_str(), p))
            .collect();

        let chain = match resolve_chain(&by_profile, &profile) {
            Ok(chain) => chain,
            Err(InheritanceError::ProfileNotFound { .. }) => {
                return Err(PolicyLookupError::Unmanaged)
            }
            Err(e) => return Err(PolicyLookupError::Internal(e.to_string())),
        };

        let leaf = chain[0];
        // Sensitive to every version in the chain, not just the leaf's own
        // (see the `cache` field's doc comment and `combined_chain_version`
        // itself) — this is what both the cache key and the served
        // `policy_version`/ETag below key off, instead of `leaf.version`
        // alone.
        let combined_version = combined_chain_version(&chain);
        let cache_key = (app.to_string(), profile.clone(), combined_version);

        if let Some(cached) = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&cache_key)
        {
            return Ok(cached.clone());
        }

        let (enforced, recommended) = flatten(&chain);
        let policy = Policy {
            contract_version: 1,
            app: app.to_string(),
            profile,
            policy_version: combined_version,
            max_cache_age_secs: leaf.max_cache_age_secs,
            stale_action: leaf.stale_action,
            enforced,
            recommended,
        };

        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(cache_key, policy.clone());

        Ok(policy)
    }

    /// The `/v1/resolve/{app}` diagnostic: which profile `claims` resolves
    /// to and which rule selected it — assignment resolution only, no
    /// policy lookup, no configuration values ever touched.
    pub async fn resolve_diagnostic(
        &self,
        app: &str,
        claims: &Value,
    ) -> Result<ResolveDiagnostic, PolicyLookupError> {
        let rules = self
            .policy_store
            .assignment_rules(app)
            .await
            .map_err(|e| PolicyLookupError::Internal(e.to_string()))?;
        let Some(resolved) = resolve_profile(&rules, claims) else {
            return Err(PolicyLookupError::Unmanaged);
        };
        let rule = resolved.rule;
        Ok(ResolveDiagnostic {
            profile: rule.profile.clone(),
            matched_rule: MatchedRule {
                ord: rule.ord,
                claim_path: (!matches!(rule.operator, super::types::RuleOperator::Default))
                    .then(|| rule.claim_path.clone()),
                operator: rule.operator,
                value: rule.value.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::service::fs_store::FsPolicyStore;
    use crate::config::service::identity::CallerIdentity;
    use crate::config::service::memory_store::InMemoryUserConfigStore;
    use async_trait::async_trait;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    struct StubIdentity;
    #[async_trait]
    impl CallerIdentity for StubIdentity {
        async fn authenticate(
            &self,
            _authorization_header: Option<&str>,
        ) -> Result<Value, super::super::error::ConfigServiceError> {
            Ok(json!({"sub": "u1"}))
        }
    }

    fn write(path: &std::path::Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn manifest_json() -> &'static str {
        r#"{"manifest_schema_version":1,"app":"myapp","fields":[{"key":"greeting","kind":"string","scope":"machine"}]}"#
    }

    async fn state_with_bundle(root: &std::path::Path) -> Arc<ConfigServiceState> {
        let store = FsPolicyStore::load(root).unwrap();
        ConfigServiceState::new(
            Arc::new(store),
            Arc::new(InMemoryUserConfigStore::new()),
            Arc::new(StubIdentity),
        )
    }

    #[tokio::test]
    async fn lookup_policy_returns_unmanaged_when_no_rule_matches() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("manifests/myapp.json"), manifest_json());
        let state = state_with_bundle(dir.path()).await;
        let err = state.lookup_policy("myapp", &json!({})).await.unwrap_err();
        assert!(matches!(err, PolicyLookupError::Unmanaged));
    }

    #[tokio::test]
    async fn lookup_policy_returns_unmanaged_when_profile_resolves_but_has_no_policy_row() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("manifests/myapp.json"), manifest_json());
        write(
            &dir.path().join("assignments.toml"),
            r#"
            [myapp]
            default_profile = "ghost-profile"
            "#,
        );
        let state = state_with_bundle(dir.path()).await;
        let err = state.lookup_policy("myapp", &json!({})).await.unwrap_err();
        assert!(matches!(err, PolicyLookupError::Unmanaged));
    }

    #[tokio::test]
    async fn lookup_policy_returns_flattened_policy_for_the_resolved_profile() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("manifests/myapp.json"), manifest_json());
        write(
            &dir.path().join("policies/myapp/developers.toml"),
            r#"
            version = 5
            [enforced]
            "greeting" = "hello developer"
            "#,
        );
        write(
            &dir.path().join("assignments.toml"),
            r#"
            [myapp]
            default_profile = "developers"
            "#,
        );
        let state = state_with_bundle(dir.path()).await;
        let policy = state.lookup_policy("myapp", &json!({})).await.unwrap();
        assert_eq!(policy.profile, "developers");
        assert_eq!(policy.policy_version, 5);
        assert_eq!(
            policy.enforced.get("greeting"),
            Some(&json!("hello developer"))
        );
        assert_eq!(policy.contract_version, 1);
    }

    #[tokio::test]
    async fn lookup_policy_caches_by_app_profile_version() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("manifests/myapp.json"), manifest_json());
        write(
            &dir.path().join("policies/myapp/developers.toml"),
            r#"
            version = 1
            [enforced]
            "greeting" = "first"
            "#,
        );
        write(
            &dir.path().join("assignments.toml"),
            r#"
            [myapp]
            default_profile = "developers"
            "#,
        );
        let state = state_with_bundle(dir.path()).await;
        let first = state.lookup_policy("myapp", &json!({})).await.unwrap();
        assert_eq!(first.enforced.get("greeting"), Some(&json!("first")));

        // A second call must be served from cache and produce the identical
        // value even though nothing in this test mutates the backing store
        // (FsPolicyStore is read-only regardless, but the assertion here is
        // about the cache path being exercised at all, not about detecting
        // a live change).
        let second = state.lookup_policy("myapp", &json!({})).await.unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn resolve_diagnostic_reports_profile_and_rule_with_no_configuration_values() {
        let dir = TempDir::new().unwrap();
        write(
            &dir.path().join("assignments.toml"),
            r#"
            [myapp]
            [[myapp.rules]]
            claim_path = "realm_access.roles"
            operator = "contains"
            value = "developers"
            profile = "developers"
            "#,
        );
        let state = state_with_bundle(dir.path()).await;
        let claims = json!({"realm_access": {"roles": ["developers"]}});
        let diag = state.resolve_diagnostic("myapp", &claims).await.unwrap();
        assert_eq!(diag.profile, "developers");
        assert_eq!(
            diag.matched_rule.claim_path.as_deref(),
            Some("realm_access.roles")
        );

        // No configuration values anywhere in the diagnostic's serialized form.
        let serialized = serde_json::to_value(&diag).unwrap();
        assert!(serialized.get("enforced").is_none());
        assert!(serialized.get("recommended").is_none());
    }

    #[tokio::test]
    async fn resolve_diagnostic_is_unmanaged_when_nothing_matches() {
        let dir = TempDir::new().unwrap();
        let state = state_with_bundle(dir.path()).await;
        let err = state
            .resolve_diagnostic("myapp", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, PolicyLookupError::Unmanaged));
    }

    // ── Bug 1 regression: cache/ETag must be sensitive to every version in
    // the resolved chain, not just the leaf's own ────────────────────────

    /// A minimal, directly-mutable `PolicyStore` — there is no admin write
    /// API yet (spec 023's job), so this is how a test simulates "an
    /// ancestor profile's stored policy changed underneath an already
    /// cached child" without going through a nonexistent HTTP path.
    /// `FsPolicyStore` can't do this: it loads a bundle once, into memory,
    /// read-only.
    #[derive(Default)]
    struct MutablePolicyStore {
        policies: Mutex<HashMap<(String, String), StoredPolicy>>,
        assignments: Mutex<HashMap<String, Vec<crate::config::service::types::AssignmentRule>>>,
    }

    impl MutablePolicyStore {
        fn new() -> Self {
            Self::default()
        }

        fn set_policy(&self, policy: StoredPolicy) {
            self.policies
                .lock()
                .unwrap()
                .insert((policy.app.clone(), policy.profile.clone()), policy);
        }

        fn set_assignment_rules(
            &self,
            app: &str,
            rules: Vec<crate::config::service::types::AssignmentRule>,
        ) {
            self.assignments
                .lock()
                .unwrap()
                .insert(app.to_string(), rules);
        }
    }

    #[async_trait]
    impl PolicyStore for MutablePolicyStore {
        async fn manifest(
            &self,
            _app: &str,
        ) -> Result<
            Option<crate::config::service::types::StoredManifest>,
            super::super::error::StoreError,
        > {
            Ok(None)
        }

        async fn policy(
            &self,
            app: &str,
            profile: &str,
        ) -> Result<Option<StoredPolicy>, super::super::error::StoreError> {
            Ok(self
                .policies
                .lock()
                .unwrap()
                .get(&(app.to_string(), profile.to_string()))
                .cloned())
        }

        async fn policies_for_app(
            &self,
            app: &str,
        ) -> Result<Vec<StoredPolicy>, super::super::error::StoreError> {
            Ok(self
                .policies
                .lock()
                .unwrap()
                .values()
                .filter(|p| p.app == app)
                .cloned()
                .collect())
        }

        async fn assignment_rules(
            &self,
            app: &str,
        ) -> Result<
            Vec<crate::config::service::types::AssignmentRule>,
            super::super::error::StoreError,
        > {
            Ok(self
                .assignments
                .lock()
                .unwrap()
                .get(app)
                .cloned()
                .unwrap_or_default())
        }

        async fn apps(&self) -> Result<Vec<String>, super::super::error::StoreError> {
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

    /// A stored policy carrying an explicit `greeting` value — used for the
    /// ancestor whose changing content/version the test observes flowing
    /// (or, pre-fix, failing to flow) through to the child.
    fn mutable_policy_with_greeting(
        profile: &str,
        parent: Option<&str>,
        version: u64,
        greeting: &str,
    ) -> StoredPolicy {
        let mut enforced = serde_json::Map::new();
        enforced.insert("greeting".to_string(), json!(greeting));
        StoredPolicy {
            app: "myapp".to_string(),
            profile: profile.to_string(),
            enforced,
            recommended: serde_json::Map::new(),
            parent_profile: parent.map(str::to_string),
            max_cache_age_secs: 3600,
            stale_action: crate::config::StaleAction::Warn,
            version,
        }
    }

    /// A stored policy with an empty `enforced` tree of its own — so
    /// `greeting` in the flattened result comes entirely from whatever
    /// parent it inherits from, never masked by an override of its own.
    fn mutable_policy_with_no_fields(
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
            stale_action: crate::config::StaleAction::Warn,
            version,
        }
    }

    fn default_rule_to(profile: &str) -> crate::config::service::types::AssignmentRule {
        crate::config::service::types::AssignmentRule {
            app: "myapp".to_string(),
            ord: 0,
            claim_path: String::new(),
            operator: crate::config::service::types::RuleOperator::Default,
            value: None,
            profile: profile.to_string(),
        }
    }

    /// Reproduces bug 1: before the fix, the cache key (and served ETag)
    /// were keyed on the *leaf's own* stored `version` alone. Bumping only
    /// the parent's version while the child's own row is untouched left the
    /// cache key unchanged, so a second lookup returned the stale flattened
    /// value and the served `policy_version` (the ETag basis) didn't change
    /// either. Reverting the `state.rs`/`inherit.rs` fix and rerunning this
    /// test reproduces exactly that: the second lookup would still see
    /// `"v1"` and `first.policy_version == second.policy_version`.
    #[tokio::test]
    async fn lookup_policy_cache_and_etag_are_sensitive_to_an_ancestor_version_change() {
        let store = Arc::new(MutablePolicyStore::new());
        store.set_policy(mutable_policy_with_greeting("parent", None, 1, "v1"));
        // The child's own version (7) never changes across this test, and
        // it declares no `greeting` of its own -- whatever value is
        // visible comes entirely from the parent.
        store.set_policy(mutable_policy_with_no_fields("child", Some("parent"), 7));
        store.set_assignment_rules("myapp", vec![default_rule_to("child")]);
        let state = ConfigServiceState::new(
            store.clone(),
            Arc::new(InMemoryUserConfigStore::new()),
            Arc::new(StubIdentity),
        );

        let first = state.lookup_policy("myapp", &json!({})).await.unwrap();
        assert_eq!(first.enforced.get("greeting"), Some(&json!("v1")));
        let first_version = first.policy_version;

        // Bump only the parent's stored version and content -- the child's
        // own row (ord, profile, version=7) is completely untouched.
        store.set_policy(mutable_policy_with_greeting("parent", None, 2, "v2"));

        let second = state.lookup_policy("myapp", &json!({})).await.unwrap();
        assert_eq!(
            second.enforced.get("greeting"),
            Some(&json!("v2")),
            "a parent-only version bump must not be masked by a cache keyed on the child's own version alone"
        );
        assert_ne!(
            second.policy_version, first_version,
            "the served policy_version (and therefore the ETag) must change when only an ancestor's version changes"
        );
    }

    #[tokio::test]
    async fn lookup_policy_cache_still_serves_from_cache_when_nothing_in_the_chain_changed() {
        let store = Arc::new(MutablePolicyStore::new());
        store.set_policy(mutable_policy_with_greeting("parent", None, 1, "v1"));
        store.set_policy(mutable_policy_with_no_fields("child", Some("parent"), 7));
        store.set_assignment_rules("myapp", vec![default_rule_to("child")]);
        let state = ConfigServiceState::new(
            store,
            Arc::new(InMemoryUserConfigStore::new()),
            Arc::new(StubIdentity),
        );

        let first = state.lookup_policy("myapp", &json!({})).await.unwrap();
        let second = state.lookup_policy("myapp", &json!({})).await.unwrap();
        assert_eq!(first, second, "an unchanged chain must still hit the cache");
    }
}
