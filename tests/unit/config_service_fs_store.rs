//! `FsPolicyStore`, exercised as an ordinary `Arc<dyn PolicyStore>` trait
//! object from outside the crate — the house-convention external target
//! complementing `src/config/service/fs_store.rs`'s inline suite, which
//! tests the concrete type directly. This file's job is to prove the trait
//! object story actually works for an external caller, since that's how
//! every real consumer (the router, the conformance suite) holds it.

use cli_framework::config::service::{PolicyStore, RuleOperator};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn store_from(root: &Path) -> Arc<dyn PolicyStore> {
    Arc::new(cli_framework::config::service::FsPolicyStore::load(root).unwrap())
}

#[tokio::test]
async fn works_as_a_trait_object_across_every_method() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        &root.join("manifests/myapp.json"),
        r#"{"manifest_schema_version":1,"app":"myapp","fields":[{"key":"greeting","kind":"string","scope":"machine"}]}"#,
    );
    write(&root.join("policies/myapp/base.toml"), "");
    write(
        &root.join("assignments.toml"),
        r#"
        [myapp]
        default_profile = "base"
        "#,
    );

    let store: Arc<dyn PolicyStore> = store_from(root);

    assert!(store.manifest("myapp").await.unwrap().is_some());
    assert!(store.policy("myapp", "base").await.unwrap().is_some());
    assert_eq!(store.policies_for_app("myapp").await.unwrap().len(), 1);
    assert_eq!(store.assignment_rules("myapp").await.unwrap().len(), 1);
    assert_eq!(store.apps().await.unwrap(), vec!["myapp".to_string()]);
}

#[tokio::test]
async fn explicit_rules_are_ordered_before_the_default_rule() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        &root.join("assignments.toml"),
        r#"
        [myapp]
        default_profile = "fallback"

        [[myapp.rules]]
        claim_path = "team"
        operator = "equals"
        value = "eng"
        profile = "engineering"
        "#,
    );
    let store = store_from(root);
    let rules = store.assignment_rules("myapp").await.unwrap();
    assert_eq!(rules.len(), 2);
    assert!(rules[0].ord < rules[1].ord);
    assert_eq!(rules[1].operator, RuleOperator::Default);
}

#[tokio::test]
async fn apps_reports_the_union_across_manifests_policies_and_assignments() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        &root.join("manifests/manifest-only-app.json"),
        r#"{"manifest_schema_version":1,"app":"manifest-only-app","fields":[]}"#,
    );
    write(&root.join("policies/policy-only-app/base.toml"), "");
    write(
        &root.join("assignments.toml"),
        r#"
        [assignment-only-app]
        default_profile = "p"
        "#,
    );

    let store = store_from(root);
    let mut apps = store.apps().await.unwrap();
    apps.sort();
    assert_eq!(
        apps,
        vec![
            "assignment-only-app".to_string(),
            "manifest-only-app".to_string(),
            "policy-only-app".to_string(),
        ]
    );
}
