//! Bundle export/import mechanics (spec 023): serialize the live store into
//! the exact bundle-directory format [`super::fs_store`] documents (JSON
//! manifests, TOML policies, one TOML assignments file), tar it up, and —
//! for import — the reverse: untar into a scratch directory and hand it to
//! [`super::fs_store::FsPolicyStore::load`], which is the *entire* parser
//! (no second, hand-rolled one).
//!
//! Every scratch directory this module creates is uniquely named (a
//! process-wide [`AtomicU64`] counter mixed with the PID, the same
//! collision-avoidance discipline [`crate::config::file_backend`]'s
//! `WRITE_COUNTER` already established for temp files) and is always removed
//! before the caller sees a result — success or failure — so a failed
//! export/import never leaks a scratch directory.

use super::error::StoreError;
use super::fs_store::FsPolicyStore;
use super::store::PolicyStore;
use super::types::RuleOperator;
use crate::config::StaleAction;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, never-before-used scratch directory path under the OS temp
/// directory — not yet created on disk. `label` (`"export"`/`"import"`) is
/// purely diagnostic, visible in the directory name if a caller ever needs
/// to inspect a leaked one by hand.
fn scratch_dir_path(label: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cli-framework-config-service-{label}-{}-{n}",
        std::process::id()
    ))
}

// ── Export: live store -> bundle directory -> tar bytes ────────────────────

#[derive(Debug, Serialize)]
struct PolicyFileOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_profile: Option<String>,
    version: u64,
    max_cache_age_secs: u64,
    stale_action: StaleAction,
    enforced: toml::value::Table,
    recommended: toml::value::Table,
}

#[derive(Debug, Default, Serialize)]
struct AppAssignmentsOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    default_profile: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    rules: Vec<RuleFileOut>,
}

#[derive(Debug, Serialize)]
struct RuleFileOut {
    claim_path: String,
    operator: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<toml::Value>,
    profile: String,
}

/// The whole-JSON-object-at-once conversion — `toml::value::Table` (like
/// every TOML map type) implements `Deserialize`, so this round-trips a
/// `serde_json::Map` through serde's generic data model rather than
/// converting key by key. Fails only if `map` contains something TOML
/// cannot represent (TOML has no `null`) — not reachable for a legitimately
/// *stored* policy's `enforced`/`recommended` trees, since a merge-patch
/// `null` only ever means "remove," never "store a null value."
fn json_map_to_toml_table(map: &Map<String, Value>) -> Result<toml::value::Table, String> {
    serde_json::from_value(Value::Object(map.clone())).map_err(|e| e.to_string())
}

fn json_value_to_toml_value(value: &Value) -> Result<toml::Value, String> {
    serde_json::from_value(value.clone()).map_err(|e| e.to_string())
}

fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, contents).map_err(|e| e.to_string())
}

/// Write the entire bundle-directory tree (manifests/, policies/,
/// assignments.toml) under `root`, reading every app from `policy_store`.
async fn write_export_bundle(policy_store: &dyn PolicyStore, root: &Path) -> Result<(), String> {
    let apps = policy_store.apps().await.map_err(|e| e.to_string())?;
    let mut assignment_apps: BTreeMap<String, AppAssignmentsOut> = BTreeMap::new();

    for app in &apps {
        if let Some(manifest) = policy_store
            .manifest(app)
            .await
            .map_err(|e| e.to_string())?
        {
            let manifest_json =
                serde_json::to_string_pretty(&manifest.doc).map_err(|e| e.to_string())?;
            write_file(
                &root.join("manifests").join(format!("{app}.json")),
                &manifest_json,
            )?;
        }

        let policies = policy_store
            .policies_for_app(app)
            .await
            .map_err(|e| e.to_string())?;
        for policy in &policies {
            let out = PolicyFileOut {
                parent_profile: policy.parent_profile.clone(),
                version: policy.version,
                max_cache_age_secs: policy.max_cache_age_secs,
                stale_action: policy.stale_action,
                enforced: json_map_to_toml_table(&policy.enforced)?,
                recommended: json_map_to_toml_table(&policy.recommended)?,
            };
            let toml_str = toml::to_string_pretty(&out).map_err(|e| e.to_string())?;
            write_file(
                &root
                    .join("policies")
                    .join(app)
                    .join(format!("{}.toml", policy.profile)),
                &toml_str,
            )?;
        }

        let mut rules = policy_store
            .assignment_rules(app)
            .await
            .map_err(|e| e.to_string())?;
        rules.sort_by_key(|r| r.ord);
        if !rules.is_empty() {
            let mut app_out = AppAssignmentsOut::default();
            for rule in &rules {
                if rule.operator == RuleOperator::Default {
                    // Folded back into `default_profile`, never emitted as
                    // an explicit rule -- the inverse of what
                    // `FsPolicyStore::load_assignments` does on the way in
                    // (spec 023, "Export/Import").
                    app_out.default_profile = Some(rule.profile.clone());
                } else {
                    app_out.rules.push(RuleFileOut {
                        claim_path: rule.claim_path.clone(),
                        operator: rule.operator.wire_str().to_string(),
                        value: rule
                            .value
                            .as_ref()
                            .map(json_value_to_toml_value)
                            .transpose()?,
                        profile: rule.profile.clone(),
                    });
                }
            }
            assignment_apps.insert(app.clone(), app_out);
        }
    }

    if !assignment_apps.is_empty() {
        let toml_str = toml::to_string_pretty(&assignment_apps).map_err(|e| e.to_string())?;
        write_file(&root.join("assignments.toml"), &toml_str)?;
    }

    Ok(())
}

fn tar_directory(root: &Path) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut buf);
        builder
            .append_dir_all(".", root)
            .map_err(|e| e.to_string())?;
        builder.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf)
}

/// Export the whole configuration set `policy_store` knows about as an
/// in-memory tar archive (spec 023, `GET /v1/admin/export`). Builds the
/// bundle under a fresh scratch directory, tars it, and removes the scratch
/// directory again before returning — on both the success and the error
/// path.
pub(crate) async fn build_export_tar(
    policy_store: &dyn PolicyStore,
) -> Result<Vec<u8>, StoreError> {
    build_export_tar_at(policy_store, &scratch_dir_path("export")).await
}

/// The actual work, parameterized over the scratch root — separated out
/// purely so a test can pass a dedicated, pre-existing path instead of one
/// drawn from the shared, process-wide [`SCRATCH_COUNTER`] (mirroring
/// [`extract_bundle_from_tar_at`]'s own reason for existing separately from
/// [`extract_bundle_from_tar`] — see that function's doc comment).
async fn build_export_tar_at(
    policy_store: &dyn PolicyStore,
    root: &Path,
) -> Result<Vec<u8>, StoreError> {
    let result: Result<Vec<u8>, String> = async {
        // `create_dir` (never `_all`) for this exact leaf directory,
        // deliberately: `temp_dir()` itself always already exists, but the
        // leaf must not. Fix 4 (spec 023 review, "Medium": scratch directory
        // names can collide with a stale directory from a prior crashed
        // process reusing the same PID) — silently reusing an existing
        // directory here (the old `create_dir_all` behavior) risked mixing
        // in leftover files that were never part of this export. Failing
        // loudly instead means a reused PID/counter collision surfaces as an
        // error rather than silently producing a wrong bundle.
        std::fs::create_dir(root).map_err(|e| {
            format!(
                "scratch directory {root:?} could not be created for export -- it may already \
                 exist (e.g. left over from a prior crashed process whose PID has since been \
                 reused): {e}"
            )
        })?;
        write_export_bundle(policy_store, root).await?;
        tar_directory(root)
    }
    .await;
    let _ = std::fs::remove_dir_all(root);
    result.map_err(StoreError::backend)
}

// ── Import: tar bytes -> scratch directory -> FsPolicyStore ────────────────

/// Extract `tar_bytes` into a fresh scratch directory and parse it as a
/// bundle via [`FsPolicyStore::load`] — the *entire* parser for the
/// bundle-directory format; this function does not hand-roll a second one
/// (spec 023's explicit instruction). The scratch directory is removed
/// before returning, on both the success and the error path, so a bad
/// (unparseable, or not-actually-a-tar) upload never leaks one.
pub(crate) fn extract_bundle_from_tar(tar_bytes: &[u8]) -> Result<FsPolicyStore, String> {
    extract_bundle_from_tar_at(tar_bytes, &scratch_dir_path("import"))
}

/// The actual work, parameterized over the scratch root (`pub(crate)`
/// purely so a test can pass a dedicated, otherwise-untouched path instead
/// of one drawn from the shared, process-wide [`SCRATCH_COUNTER`] — scanning
/// the *global* OS temp directory for leaks, as a test naturally would
/// otherwise, races against every other test in this binary that also
/// creates and cleans up a scratch directory concurrently. A test-owned
/// `root` sidesteps that race entirely rather than trying to out-synchronize
/// it.
fn extract_bundle_from_tar_at(tar_bytes: &[u8], root: &Path) -> Result<FsPolicyStore, String> {
    let result = (|| {
        // See `build_export_tar_at`'s matching comment (Fix 4, spec 023
        // review): `create_dir`, never `_all`, for this exact leaf so a
        // stale directory from a prior crashed process (possible PID reuse)
        // is a loud error rather than a silent read of leftover files mixed
        // into a fresh import.
        std::fs::create_dir(root).map_err(|e| {
            format!(
                "scratch directory {root:?} could not be created for import -- it may already \
                 exist (e.g. left over from a prior crashed process whose PID has since been \
                 reused): {e}"
            )
        })?;
        let mut archive = tar::Archive::new(tar_bytes);
        archive.unpack(root).map_err(|e| e.to_string())?;
        FsPolicyStore::load(root).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_dir_all(root);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
    use crate::config::service::types::{AssignmentRule, StoredManifest, StoredPolicy};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;

    /// A minimal, directly-constructible `PolicyStore` for exercising
    /// export without needing a real backend.
    #[derive(Default)]
    struct FixturePolicyStore {
        manifests: HashMap<String, StoredManifest>,
        policies: HashMap<String, Vec<StoredPolicy>>,
        assignments: HashMap<String, Vec<AssignmentRule>>,
    }

    #[async_trait]
    impl PolicyStore for FixturePolicyStore {
        async fn manifest(&self, app: &str) -> Result<Option<StoredManifest>, StoreError> {
            Ok(self.manifests.get(app).cloned())
        }
        async fn policy(
            &self,
            app: &str,
            profile: &str,
        ) -> Result<Option<StoredPolicy>, StoreError> {
            Ok(self
                .policies
                .get(app)
                .and_then(|ps| ps.iter().find(|p| p.profile == profile))
                .cloned())
        }
        async fn policies_for_app(&self, app: &str) -> Result<Vec<StoredPolicy>, StoreError> {
            Ok(self.policies.get(app).cloned().unwrap_or_default())
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

    fn sample_manifest(app: &str) -> StoredManifest {
        StoredManifest {
            app: app.to_string(),
            doc: ConfigManifest::new(
                app,
                vec![FieldManifest {
                    key: "greeting".to_string(),
                    kind: FieldKind::Str,
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
                }],
            ),
            version: 1,
        }
    }

    fn sample_policy(app: &str, profile: &str, parent: Option<&str>) -> StoredPolicy {
        let mut enforced = Map::new();
        enforced.insert("greeting".to_string(), json!("hi"));
        StoredPolicy {
            app: app.to_string(),
            profile: profile.to_string(),
            enforced,
            recommended: Map::new(),
            parent_profile: parent.map(str::to_string),
            max_cache_age_secs: 3600,
            stale_action: StaleAction::Warn,
            version: 3,
        }
    }

    #[tokio::test]
    async fn export_then_reload_via_fspolicystore_reproduces_the_same_documents() {
        let mut store = FixturePolicyStore::default();
        store
            .manifests
            .insert("myapp".to_string(), sample_manifest("myapp"));
        store.policies.insert(
            "myapp".to_string(),
            vec![
                sample_policy("myapp", "base", None),
                sample_policy("myapp", "developers", Some("base")),
            ],
        );
        store.assignments.insert(
            "myapp".to_string(),
            vec![
                AssignmentRule {
                    app: "myapp".to_string(),
                    ord: 0,
                    claim_path: "realm_access.roles".to_string(),
                    operator: RuleOperator::Contains,
                    value: Some(json!("developers")),
                    profile: "developers".to_string(),
                },
                AssignmentRule {
                    app: "myapp".to_string(),
                    ord: 1,
                    claim_path: String::new(),
                    operator: RuleOperator::Default,
                    value: None,
                    profile: "base".to_string(),
                },
            ],
        );

        let tar_bytes = build_export_tar(&store).await.unwrap();
        assert!(!tar_bytes.is_empty());

        let reloaded = extract_bundle_from_tar(&tar_bytes).unwrap();

        let manifest = reloaded.manifest("myapp").await.unwrap().unwrap();
        assert_eq!(manifest.doc.app, "myapp");

        let base = reloaded.policy("myapp", "base").await.unwrap().unwrap();
        assert_eq!(base.enforced.get("greeting"), Some(&json!("hi")));
        assert!(base.parent_profile.is_none());

        let developers = reloaded
            .policy("myapp", "developers")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(developers.parent_profile.as_deref(), Some("base"));

        let rules = reloaded.assignment_rules("myapp").await.unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].operator, RuleOperator::Contains);
        assert_eq!(rules[1].operator, RuleOperator::Default);
        assert_eq!(rules[1].profile, "base");
    }

    #[tokio::test]
    async fn export_of_an_empty_store_produces_a_loadable_empty_bundle() {
        let store = FixturePolicyStore::default();
        let tar_bytes = build_export_tar(&store).await.unwrap();
        let reloaded = extract_bundle_from_tar(&tar_bytes).unwrap();
        assert!(reloaded.apps().await.unwrap().is_empty());
    }

    /// Uses [`extract_bundle_from_tar_at`] with a dedicated, test-owned
    /// scratch root (inside a [`tempfile::TempDir`] that is itself cleaned
    /// up by its own `Drop` at the end of the test) rather than
    /// [`extract_bundle_from_tar`]'s process-wide counter-based path —
    /// scanning the *shared* OS temp directory for leaks would race against
    /// every other `#[tokio::test]`/`#[test]` in this binary doing the same
    /// thing concurrently (`cargo test` runs tests in parallel by default),
    /// which is exactly what made an earlier version of this test flaky.
    #[test]
    fn extracting_garbage_bytes_as_a_tar_archive_is_an_error_and_leaves_no_scratch_dir() {
        let base = tempfile::TempDir::new().unwrap();
        // `root` itself must not exist yet -- matching `scratch_dir_path`'s
        // own contract of returning a fresh, not-yet-created path.
        let root = base.path().join("scratch-root");
        assert!(!root.exists());

        let err =
            extract_bundle_from_tar_at(b"this is not a tar archive at all", &root).unwrap_err();
        assert!(!err.is_empty());
        assert!(
            !root.exists(),
            "a failed extraction must not leak its scratch directory"
        );
    }

    /// Fix 4 (spec 023 review, "Medium": scratch directory names can
    /// collide with a stale directory from a prior crashed process reusing
    /// the same PID): pre-creates the exact leaf path `build_export_tar_at`
    /// is handed, simulating that stale-directory collision, and confirms
    /// the call fails loudly instead of silently reusing it. Before the fix
    /// (`std::fs::create_dir_all`, which succeeds unconditionally whether or
    /// not the directory already existed), this exact scenario would
    /// silently proceed to build and return a bundle rather than erroring —
    /// confirmed by hand while writing this test (temporarily reverting to
    /// `create_dir_all` here made this assertion fail with "called
    /// `Result::unwrap_err()` on an `Ok` value", then restoring the fix made
    /// it pass again).
    #[tokio::test]
    async fn build_export_tar_at_fails_loudly_if_the_scratch_directory_already_exists() {
        let base = tempfile::TempDir::new().unwrap();
        let root = base.path().join("scratch-root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("stale-leftover-file.txt"), b"leftover").unwrap();

        let store = FixturePolicyStore::default();
        let err = build_export_tar_at(&store, &root).await.unwrap_err();
        assert!(
            matches!(err, StoreError::Backend(_)),
            "a pre-existing scratch directory must be a loud error, not a silent reuse: {err:?}"
        );
    }

    /// The `extract_bundle_from_tar_at` (import-side) counterpart to
    /// [`build_export_tar_at_fails_loudly_if_the_scratch_directory_already_exists`] —
    /// same collision scenario, same negative-check performed by hand
    /// (temporarily reverting to `create_dir_all` here also made this
    /// assertion fail, confirming the test can actually detect the bug it's
    /// named for).
    #[tokio::test]
    async fn extract_bundle_from_tar_at_fails_loudly_if_the_scratch_directory_already_exists() {
        let store = FixturePolicyStore::default();
        let tar_bytes = build_export_tar(&store).await.unwrap();

        let base = tempfile::TempDir::new().unwrap();
        let root = base.path().join("scratch-root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("stale-leftover-file.txt"), b"leftover").unwrap();

        let err = extract_bundle_from_tar_at(&tar_bytes, &root).unwrap_err();
        assert!(
            !err.is_empty(),
            "a pre-existing scratch directory must be a loud error, not a silent reuse"
        );
    }

    #[test]
    fn scratch_dir_path_is_unique_across_calls() {
        let a = scratch_dir_path("test");
        let b = scratch_dir_path("test");
        assert_ne!(a, b);
    }

    #[test]
    fn json_map_to_toml_table_round_trips_scalars() {
        let mut map = Map::new();
        map.insert("a".to_string(), json!("x"));
        map.insert("b".to_string(), json!(5));
        map.insert("c".to_string(), json!(true));
        let table = json_map_to_toml_table(&map).unwrap();
        assert_eq!(table.get("a").unwrap().as_str(), Some("x"));
        assert_eq!(table.get("b").unwrap().as_integer(), Some(5));
        assert_eq!(table.get("c").unwrap().as_bool(), Some(true));
    }
}
