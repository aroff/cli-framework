//! `#[derive(ConfigManifest)]` — spec 021 testing decisions: "a struct with
//! every flag combination produces the expected manifest document, checked
//! as a value, not a byte string."
//!
//! `AppConfig` below exercises every flag in the ADR 0073 flag list at least
//! once (`scope` in all three values, `platforms`, `secret`, `local_only`,
//! `protected`, `manageable`, `enforceable`, `restart_required`), every
//! `FieldKind` (including a hand-inferred `duration`/`url`/`enum` override
//! and a nested `section`), and both constraint kinds (range, allowed
//! values).

use cli_framework::config::manifest::{
    ConfigManifest as ConfigManifestDoc, FieldConstraints, FieldKind, FieldManifest,
    IntoConfigManifest, Scope, MANIFEST_SCHEMA_VERSION,
};
use cli_framework::ConfigManifest;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, ConfigManifest)]
#[config_manifest(app = "network-section")]
struct NetworkConfig {
    #[manifest(kind = "url", label = "Proxy URL", scope = "machine")]
    proxy_url: String,
    #[manifest(local_only, description = "Fixed at install time")]
    listen_port: u32,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            proxy_url: "http://proxy.internal:3128".to_string(),
            listen_port: 8080,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, ConfigManifest)]
#[config_manifest(app = "myapp")]
struct AppConfig {
    #[manifest(
        label = "Telemetry enabled",
        description = "Send anonymous usage statistics",
        group = "privacy",
        scope = "user"
    )]
    telemetry_enabled: bool,

    #[manifest(scope = "org", protected)]
    compliance_mode: bool,

    #[manifest(secret)]
    api_token: String,

    #[manifest(local_only)]
    service_endpoint: String,

    #[manifest(manageable = false)]
    experimental_feature: bool,

    #[manifest(enforceable = false)]
    telemetry_opt_in: bool,

    #[manifest(restart_required)]
    worker_threads: u32,

    #[manifest(kind = "duration", label = "Poll interval")]
    poll_interval_secs: u64,

    #[manifest(kind = "enum", allowed = "low,medium,high", label = "Log level")]
    log_level: String,

    #[manifest(min = 1, max = 100)]
    retry_limit: u32,

    tags: Vec<String>,

    #[manifest(platforms = "desktop,mobile")]
    show_banner: bool,

    #[manifest(key = "proxy", section)]
    network: NetworkConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            telemetry_enabled: true,
            compliance_mode: false,
            api_token: String::new(),
            service_endpoint: "https://api.internal".to_string(),
            experimental_feature: false,
            telemetry_opt_in: false,
            worker_threads: 4,
            poll_interval_secs: 30,
            log_level: "medium".to_string(),
            retry_limit: 3,
            tags: vec!["default".to_string()],
            show_banner: true,
            network: NetworkConfig::default(),
        }
    }
}

fn expected_network_fields() -> Vec<FieldManifest> {
    vec![
        FieldManifest {
            key: "proxy_url".to_string(),
            kind: FieldKind::Url,
            default: Some(serde_json::json!("http://proxy.internal:3128")),
            label: Some("Proxy URL".to_string()),
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
        },
        FieldManifest {
            key: "listen_port".to_string(),
            kind: FieldKind::Int,
            default: Some(serde_json::json!(8080)),
            label: None,
            description: Some("Fixed at install time".to_string()),
            group: None,
            scope: Scope::Machine,
            platforms: vec![],
            secret: false,
            local_only: true,
            protected: false,
            manageable: true,
            enforceable: true,
            restart_required: false,
            constraints: None,
        },
    ]
}

fn expected_manifest() -> ConfigManifestDoc {
    ConfigManifestDoc::new(
        "myapp",
        vec![
            FieldManifest {
                key: "telemetry_enabled".to_string(),
                kind: FieldKind::Bool,
                default: Some(serde_json::json!(true)),
                label: Some("Telemetry enabled".to_string()),
                description: Some("Send anonymous usage statistics".to_string()),
                group: Some("privacy".to_string()),
                scope: Scope::User,
                platforms: vec![],
                secret: false,
                local_only: false,
                protected: false,
                manageable: true,
                enforceable: true,
                restart_required: false,
                constraints: None,
            },
            FieldManifest {
                key: "compliance_mode".to_string(),
                kind: FieldKind::Bool,
                default: Some(serde_json::json!(false)),
                label: None,
                description: None,
                group: None,
                scope: Scope::Org,
                platforms: vec![],
                secret: false,
                local_only: false,
                protected: true,
                manageable: true,
                enforceable: true,
                restart_required: false,
                constraints: None,
            },
            FieldManifest {
                key: "api_token".to_string(),
                kind: FieldKind::Str,
                default: Some(serde_json::json!("")),
                label: None,
                description: None,
                group: None,
                scope: Scope::Machine,
                platforms: vec![],
                secret: true,
                local_only: false,
                protected: false,
                manageable: true,
                enforceable: true,
                restart_required: false,
                constraints: None,
            },
            FieldManifest {
                key: "service_endpoint".to_string(),
                kind: FieldKind::Str,
                default: Some(serde_json::json!("https://api.internal")),
                label: None,
                description: None,
                group: None,
                scope: Scope::Machine,
                platforms: vec![],
                secret: false,
                local_only: true,
                protected: false,
                manageable: true,
                enforceable: true,
                restart_required: false,
                constraints: None,
            },
            FieldManifest {
                key: "experimental_feature".to_string(),
                kind: FieldKind::Bool,
                default: Some(serde_json::json!(false)),
                label: None,
                description: None,
                group: None,
                scope: Scope::Machine,
                platforms: vec![],
                secret: false,
                local_only: false,
                protected: false,
                manageable: false,
                enforceable: true,
                restart_required: false,
                constraints: None,
            },
            FieldManifest {
                key: "telemetry_opt_in".to_string(),
                kind: FieldKind::Bool,
                default: Some(serde_json::json!(false)),
                label: None,
                description: None,
                group: None,
                scope: Scope::Machine,
                platforms: vec![],
                secret: false,
                local_only: false,
                protected: false,
                manageable: true,
                enforceable: false,
                restart_required: false,
                constraints: None,
            },
            FieldManifest {
                key: "worker_threads".to_string(),
                kind: FieldKind::Int,
                default: Some(serde_json::json!(4)),
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
                restart_required: true,
                constraints: None,
            },
            FieldManifest {
                key: "poll_interval_secs".to_string(),
                kind: FieldKind::Duration,
                default: Some(serde_json::json!(30)),
                label: Some("Poll interval".to_string()),
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
            },
            FieldManifest {
                key: "log_level".to_string(),
                kind: FieldKind::Enum {
                    values: vec!["low".to_string(), "medium".to_string(), "high".to_string()],
                },
                default: Some(serde_json::json!("medium")),
                label: Some("Log level".to_string()),
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
            },
            FieldManifest {
                key: "retry_limit".to_string(),
                kind: FieldKind::Int,
                default: Some(serde_json::json!(3)),
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
                constraints: Some(FieldConstraints {
                    min: Some(1.0),
                    max: Some(100.0),
                    allowed_values: None,
                }),
            },
            FieldManifest {
                key: "tags".to_string(),
                kind: FieldKind::List {
                    item: Box::new(FieldKind::Str),
                },
                default: Some(serde_json::json!(["default"])),
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
            },
            FieldManifest {
                key: "show_banner".to_string(),
                kind: FieldKind::Bool,
                default: Some(serde_json::json!(true)),
                label: None,
                description: None,
                group: None,
                scope: Scope::Machine,
                platforms: vec!["desktop".to_string(), "mobile".to_string()],
                secret: false,
                local_only: false,
                protected: false,
                manageable: true,
                enforceable: true,
                restart_required: false,
                constraints: None,
            },
            FieldManifest {
                key: "proxy".to_string(),
                kind: FieldKind::Section {
                    fields: expected_network_fields(),
                },
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
            },
        ],
    )
}

#[test]
fn derive_produces_the_expected_manifest_value_for_every_flag_combination() {
    let manifest = AppConfig::config_manifest();
    assert_eq!(manifest, expected_manifest());
}

#[test]
fn manifest_schema_version_is_stamped() {
    let manifest = AppConfig::config_manifest();
    assert_eq!(manifest.manifest_schema_version, MANIFEST_SCHEMA_VERSION);
}

#[test]
fn nested_section_flattens_under_its_own_key_prefix() {
    let manifest = AppConfig::config_manifest();
    let paths: Vec<String> = manifest.iter_leaves().into_iter().map(|l| l.path).collect();
    assert!(paths.contains(&"proxy.proxy_url".to_string()));
    assert!(paths.contains(&"proxy.listen_port".to_string()));
    assert!(
        !paths.contains(&"proxy".to_string()),
        "a section is not itself a leaf"
    );
}

#[test]
fn manifest_is_a_plain_document_that_round_trips_through_json() {
    // Spec 021: "every runtime consumer... reads the JSON document alone."
    // Prove the derive's output really is that document by round-tripping
    // it through JSON with no special-cased deserialization.
    let manifest = AppConfig::config_manifest();
    let json = serde_json::to_value(&manifest).unwrap();
    let back: ConfigManifestDoc = serde_json::from_value(json).unwrap();
    assert_eq!(back, manifest);
}

#[test]
fn each_policy_flag_is_independently_observable() {
    let manifest = AppConfig::config_manifest();
    let field = |key: &str| manifest.fields.iter().find(|f| f.key == key).unwrap();

    assert_eq!(field("telemetry_enabled").scope, Scope::User);
    assert_eq!(field("compliance_mode").scope, Scope::Org);
    assert!(field("compliance_mode").protected);
    assert!(field("api_token").secret);
    assert!(field("service_endpoint").local_only);
    assert!(!field("experimental_feature").manageable);
    assert!(!field("telemetry_opt_in").enforceable);
    assert!(field("worker_threads").restart_required);
    assert_eq!(field("show_banner").platforms, vec!["desktop", "mobile"]);
}

#[test]
fn json_schema_export_is_consumable_for_the_derived_manifest() {
    let manifest = AppConfig::config_manifest();
    let schema = cli_framework::config::manifest::to_json_schema(&manifest);
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["properties"]["log_level"]["enum"],
        serde_json::json!(["low", "medium", "high"])
    );
    assert_eq!(
        schema["properties"]["proxy"]["properties"]["proxy_url"]["type"],
        "string"
    );
}
