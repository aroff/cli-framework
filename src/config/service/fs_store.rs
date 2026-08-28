//! [`FsPolicyStore`]: a read-only, bundle-directory [`PolicyStore`] for
//! tests and local development (spec 022, "Bundle format" / user story 30:
//! "run and test the service without Postgres").
//!
//! Everything is loaded into memory once, at [`FsPolicyStore::load`] —
//! there is no live filesystem watching and no write path; the bundle
//! directory is an import/export/seed/test-fixture format only, never a
//! thing the service polls (spec 022: "The service never reads a Git
//! repository and no reconciliation loop exists" — the same principle
//! applies to this plain-directory form).
//!
//! Layout, rooted at a directory of the caller's choosing:
//!
//! ```text
//! <root>/
//!   manifests/
//!     <app>.json          -- a ConfigManifest document, verbatim
//!   policies/
//!     <app>/
//!       <profile>.toml    -- one stored policy
//!   assignments.toml      -- every app's ordered assignment rules
//! ```
//!
//! `policies/<app>/<profile>.toml`:
//!
//! ```toml
//! parent_profile = "base"        # optional
//! version = 2                    # optional, defaults to 1
//! max_cache_age_secs = 3600      # optional, defaults to 3600
//! stale_action = "warn"          # optional, "warn" | "refuse"
//!
//! [enforced]
//! "network.proxy_url" = "http://proxy.example.com"
//!
//! [recommended]
//! "telemetry.enabled" = true
//! ```
//!
//! Note the **quoted, dotted** keys under `[enforced]`/`[recommended]` —
//! these trees are flat, dotted-leaf-path-keyed maps (the same coordinate
//! system [`crate::config::Policy`]'s wire trees use), never a nested table
//! mirroring the manifest's own section structure.
//!
//! `assignments.toml`, one top-level table per app:
//!
//! ```toml
//! [myapp]
//! default_profile = "kiosk"      # optional
//!
//! [[myapp.rules]]
//! claim_path = "realm_access.roles"
//! operator = "contains"          # "equals" | "contains" | "exists"
//! value = "developers"
//! profile = "developers"
//! ```
//!
//! A `default_profile`, if present, becomes a trailing
//! [`super::types::RuleOperator::Default`] rule ordered after every
//! explicit rule — see that variant's docs for why this is how "optional
//! default profile" is represented within spec 022's exact given
//! `assignment(app, ord, claim_path, operator, value, profile)` schema.

use super::error::StoreError;
use super::store::PolicyStore;
use super::types::{AssignmentRule, RuleOperator, StoredManifest, StoredPolicy};
use crate::config::manifest::ConfigManifest;
use crate::config::StaleAction;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize)]
struct AssignmentsFile {
    #[serde(flatten)]
    apps: HashMap<String, AppAssignments>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AppAssignments {
    #[serde(default)]
    default_profile: Option<String>,
    #[serde(default)]
    rules: Vec<RuleFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuleFile {
    claim_path: String,
    operator: String,
    #[serde(default)]
    value: Option<toml::Value>,
    profile: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PolicyFile {
    #[serde(default)]
    parent_profile: Option<String>,
    #[serde(default = "default_version")]
    version: u64,
    #[serde(default = "default_cache_age")]
    max_cache_age_secs: u64,
    #[serde(default = "default_stale_action")]
    stale_action: StaleAction,
    #[serde(default)]
    enforced: toml::value::Table,
    #[serde(default)]
    recommended: toml::value::Table,
}

fn default_version() -> u64 {
    1
}

fn default_cache_age() -> u64 {
    3600
}

fn default_stale_action() -> StaleAction {
    StaleAction::Warn
}

fn toml_table_to_json_map(table: toml::value::Table) -> Result<Map<String, Value>, StoreError> {
    let mut out = Map::new();
    for (k, v) in table {
        let json = serde_json::to_value(v).map_err(|e| {
            StoreError::backend(format!(
                "bundle TOML value for '{k}' could not be converted to JSON: {e}"
            ))
        })?;
        out.insert(k, json);
    }
    Ok(out)
}

/// The bundle format's own operator whitelist is deliberately narrower than
/// [`RuleOperator::parse_wire_str`]'s full set: `"default"` is rejected here
/// even though it's a valid wire string (Postgres storage does write it,
/// for a trailing default-profile row — see [`RuleOperator::Default`]'s
/// docs), because the bundle format's documented way to declare a default
/// profile is the separate `default_profile` key, not a hand-authored
/// `operator = "default"` rule.
fn parse_operator(app: &str, raw: &str) -> Result<RuleOperator, StoreError> {
    match RuleOperator::parse_wire_str(raw) {
        Some(RuleOperator::Default) | None => Err(StoreError::backend(format!(
            "bundle assignments.toml: app '{app}' has an unknown rule operator '{raw}' (expected equals|contains|exists)"
        ))),
        Some(op) => Ok(op),
    }
}

/// A read-only [`PolicyStore`] backed by a bundle directory, loaded
/// entirely into memory at construction time. See the module docs for the
/// exact directory layout.
#[derive(Debug)]
pub struct FsPolicyStore {
    manifests: HashMap<String, StoredManifest>,
    policies: HashMap<String, HashMap<String, StoredPolicy>>,
    assignments: HashMap<String, Vec<AssignmentRule>>,
}

impl FsPolicyStore {
    /// Load a bundle directory rooted at `root`. Every file under
    /// `manifests/` and `policies/` is read and parsed eagerly; a missing
    /// `manifests/`, `policies/`, or `assignments.toml` is treated as
    /// "nothing declared there" rather than an error, so a bundle can
    /// legitimately contain only some of the three.
    pub fn load(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = root.as_ref();
        let manifests = Self::load_manifests(root)?;
        let policies = Self::load_policies(root)?;
        let assignments = Self::load_assignments(root)?;
        Ok(Self {
            manifests,
            policies,
            assignments,
        })
    }

    fn load_manifests(root: &Path) -> Result<HashMap<String, StoredManifest>, StoreError> {
        let dir = root.join("manifests");
        let mut out = HashMap::new();
        if !dir.is_dir() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| StoreError::backend(format!("reading {}: {e}", dir.display())))?
        {
            let entry = entry.map_err(|e| StoreError::backend(e.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let app = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| {
                    StoreError::backend(format!("non-UTF-8 manifest filename: {}", path.display()))
                })?
                .to_string();
            let bytes = std::fs::read(&path)
                .map_err(|e| StoreError::backend(format!("reading {}: {e}", path.display())))?;
            let doc: ConfigManifest =
                serde_json::from_slice(&bytes).map_err(|e| StoreError::Corrupt {
                    app: app.clone(),
                    message: format!("{}: {e}", path.display()),
                })?;
            out.insert(
                app.clone(),
                StoredManifest {
                    app,
                    doc,
                    version: 1,
                },
            );
        }
        Ok(out)
    }

    fn load_policies(
        root: &Path,
    ) -> Result<HashMap<String, HashMap<String, StoredPolicy>>, StoreError> {
        let dir = root.join("policies");
        let mut out: HashMap<String, HashMap<String, StoredPolicy>> = HashMap::new();
        if !dir.is_dir() {
            return Ok(out);
        }
        for app_entry in std::fs::read_dir(&dir)
            .map_err(|e| StoreError::backend(format!("reading {}: {e}", dir.display())))?
        {
            let app_entry = app_entry.map_err(|e| StoreError::backend(e.to_string()))?;
            let app_path = app_entry.path();
            if !app_path.is_dir() {
                continue;
            }
            let app = app_path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| {
                    StoreError::backend(format!("non-UTF-8 app directory: {}", app_path.display()))
                })?
                .to_string();

            let mut profiles = HashMap::new();
            for profile_entry in std::fs::read_dir(&app_path)
                .map_err(|e| StoreError::backend(format!("reading {}: {e}", app_path.display())))?
            {
                let profile_entry =
                    profile_entry.map_err(|e| StoreError::backend(e.to_string()))?;
                let profile_path = profile_entry.path();
                if profile_path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                let profile = profile_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| {
                        StoreError::backend(format!(
                            "non-UTF-8 profile filename: {}",
                            profile_path.display()
                        ))
                    })?
                    .to_string();
                let text = std::fs::read_to_string(&profile_path).map_err(|e| {
                    StoreError::backend(format!("reading {}: {e}", profile_path.display()))
                })?;
                let parsed: PolicyFile =
                    toml::from_str(&text).map_err(|e| StoreError::Corrupt {
                        app: app.clone(),
                        message: format!("{}: {e}", profile_path.display()),
                    })?;
                let enforced = toml_table_to_json_map(parsed.enforced)?;
                let recommended = toml_table_to_json_map(parsed.recommended)?;
                profiles.insert(
                    profile.clone(),
                    StoredPolicy {
                        app: app.clone(),
                        profile,
                        enforced,
                        recommended,
                        parent_profile: parsed.parent_profile,
                        max_cache_age_secs: parsed.max_cache_age_secs,
                        stale_action: parsed.stale_action,
                        version: parsed.version,
                    },
                );
            }
            out.insert(app, profiles);
        }
        Ok(out)
    }

    fn load_assignments(root: &Path) -> Result<HashMap<String, Vec<AssignmentRule>>, StoreError> {
        let path = root.join("assignments.toml");
        let mut out = HashMap::new();
        if !path.is_file() {
            return Ok(out);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| StoreError::backend(format!("reading {}: {e}", path.display())))?;
        let parsed: AssignmentsFile = toml::from_str(&text)
            .map_err(|e| StoreError::backend(format!("{}: {e}", path.display())))?;

        for (app, app_rules) in parsed.apps {
            let mut rules = Vec::new();
            for (ord, rule) in app_rules.rules.into_iter().enumerate() {
                let operator = parse_operator(&app, &rule.operator)?;
                let value = match rule.value {
                    Some(v) => Some(serde_json::to_value(v).map_err(|e| {
                        StoreError::backend(format!(
                            "bundle assignments.toml: app '{app}' rule {ord} value could not be converted to JSON: {e}"
                        ))
                    })?),
                    None => None,
                };
                rules.push(AssignmentRule {
                    app: app.clone(),
                    ord: ord as i64,
                    claim_path: rule.claim_path,
                    operator,
                    value,
                    profile: rule.profile,
                });
            }
            if let Some(default_profile) = app_rules.default_profile {
                rules.push(AssignmentRule {
                    app: app.clone(),
                    ord: rules.len() as i64,
                    claim_path: String::new(),
                    operator: RuleOperator::Default,
                    value: None,
                    profile: default_profile,
                });
            }
            out.insert(app, rules);
        }
        Ok(out)
    }
}

impl FsPolicyStore {
    /// Whether `app` has an explicit `assignments.toml` stanza in this
    /// bundle, as opposed to no stanza at all. [`Self::assignment_rules`]
    /// (the [`PolicyStore`] trait method) collapses both cases to an empty
    /// `Vec` and so cannot distinguish them — callers that need to tell
    /// "declared, zero rules" apart from "not declared" (import, notably:
    /// see [`super::postgres::PgPolicyStore::import_bundle`]'s own doc
    /// comment) must use this instead.
    pub(crate) fn has_declared_assignments(&self, app: &str) -> bool {
        self.assignments.contains_key(app)
    }
}

#[async_trait]
impl PolicyStore for FsPolicyStore {
    async fn manifest(&self, app: &str) -> Result<Option<StoredManifest>, StoreError> {
        Ok(self.manifests.get(app).cloned())
    }

    async fn policy(&self, app: &str, profile: &str) -> Result<Option<StoredPolicy>, StoreError> {
        Ok(self
            .policies
            .get(app)
            .and_then(|profiles| profiles.get(profile))
            .cloned())
    }

    async fn policies_for_app(&self, app: &str) -> Result<Vec<StoredPolicy>, StoreError> {
        Ok(self
            .policies
            .get(app)
            .map(|profiles| profiles.values().cloned().collect())
            .unwrap_or_default())
    }

    async fn assignment_rules(&self, app: &str) -> Result<Vec<AssignmentRule>, StoreError> {
        Ok(self.assignments.get(app).cloned().unwrap_or_default())
    }

    async fn apps(&self) -> Result<Vec<String>, StoreError> {
        let mut apps: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        apps.extend(self.manifests.keys().cloned());
        apps.extend(self.policies.keys().cloned());
        apps.extend(self.assignments.keys().cloned());
        Ok(apps.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn sample_manifest_json() -> &'static str {
        r#"{
            "manifest_schema_version": 1,
            "app": "myapp",
            "fields": [
                {"key": "greeting", "kind": "string", "scope": "machine"},
                {"key": "proxy_url", "kind": "url", "scope": "machine"}
            ]
        }"#
    }

    #[tokio::test]
    async fn loads_manifest_policy_and_assignments_from_a_bundle() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        write(&root.join("manifests/myapp.json"), sample_manifest_json());
        write(
            &root.join("policies/myapp/developers.toml"),
            r#"
            version = 3
            max_cache_age_secs = 120
            stale_action = "refuse"

            [enforced]
            "proxy_url" = "http://proxy.example.com"

            [recommended]
            "greeting" = "hi"
            "#,
        );
        write(
            &root.join("assignments.toml"),
            r#"
            [myapp]
            default_profile = "kiosk"

            [[myapp.rules]]
            claim_path = "realm_access.roles"
            operator = "contains"
            value = "developers"
            profile = "developers"
            "#,
        );

        let store = FsPolicyStore::load(root).unwrap();

        let manifest = store.manifest("myapp").await.unwrap().unwrap();
        assert_eq!(manifest.doc.app, "myapp");

        let policy = store.policy("myapp", "developers").await.unwrap().unwrap();
        assert_eq!(policy.version, 3);
        assert_eq!(policy.max_cache_age_secs, 120);
        assert_eq!(policy.stale_action, StaleAction::Refuse);
        assert_eq!(
            policy.enforced.get("proxy_url"),
            Some(&Value::String("http://proxy.example.com".to_string()))
        );
        assert_eq!(
            policy.recommended.get("greeting"),
            Some(&Value::String("hi".to_string()))
        );

        let rules = store.assignment_rules("myapp").await.unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].operator, RuleOperator::Contains);
        assert_eq!(rules[1].operator, RuleOperator::Default);
        assert_eq!(rules[1].profile, "kiosk");

        let apps = store.apps().await.unwrap();
        assert_eq!(apps, vec!["myapp".to_string()]);
    }

    #[tokio::test]
    async fn missing_pieces_of_the_bundle_are_empty_not_errors() {
        let dir = TempDir::new().unwrap();
        let store = FsPolicyStore::load(dir.path()).unwrap();
        assert!(store.manifest("ghost").await.unwrap().is_none());
        assert!(store.policy("ghost", "p").await.unwrap().is_none());
        assert!(store.assignment_rules("ghost").await.unwrap().is_empty());
        assert!(store.apps().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn policy_without_optional_fields_gets_defaults() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(&root.join("manifests/myapp.json"), sample_manifest_json());
        write(&root.join("policies/myapp/base.toml"), "");

        let store = FsPolicyStore::load(root).unwrap();
        let policy = store.policy("myapp", "base").await.unwrap().unwrap();
        assert_eq!(policy.version, 1);
        assert_eq!(policy.max_cache_age_secs, 3600);
        assert_eq!(policy.stale_action, StaleAction::Warn);
        assert!(policy.parent_profile.is_none());
        assert!(policy.enforced.is_empty());
        assert!(policy.recommended.is_empty());
    }

    #[tokio::test]
    async fn parent_profile_is_carried_through() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(
            &root.join("policies/myapp/child.toml"),
            r#"parent_profile = "base""#,
        );
        let store = FsPolicyStore::load(root).unwrap();
        let policy = store.policy("myapp", "child").await.unwrap().unwrap();
        assert_eq!(policy.parent_profile.as_deref(), Some("base"));
    }

    #[tokio::test]
    async fn unknown_operator_in_assignments_is_a_load_error() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(
            &root.join("assignments.toml"),
            r#"
            [myapp]
            [[myapp.rules]]
            claim_path = "team"
            operator = "startswith"
            value = "x"
            profile = "p"
            "#,
        );
        let err = FsPolicyStore::load(root).unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)));
    }

    #[tokio::test]
    async fn malformed_manifest_json_is_a_corrupt_error() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(&root.join("manifests/myapp.json"), "{ not valid json");
        let err = FsPolicyStore::load(root).unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { .. }));
    }

    #[tokio::test]
    async fn policies_for_app_returns_every_profile() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(&root.join("policies/myapp/base.toml"), "");
        write(
            &root.join("policies/myapp/child.toml"),
            r#"parent_profile = "base""#,
        );
        let store = FsPolicyStore::load(root).unwrap();
        let mut profiles: Vec<String> = store
            .policies_for_app("myapp")
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.profile)
            .collect();
        profiles.sort();
        assert_eq!(profiles, vec!["base".to_string(), "child".to_string()]);
    }
}
