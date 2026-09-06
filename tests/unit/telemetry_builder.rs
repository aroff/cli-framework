// tests/unit/telemetry_builder.rs
//
// Corrected against `origin/main` per the PR7 brief's C1/C4 corrections and
// two additional defects found while transcribing:
//   - `AppBuilder` is not re-exported at the crate root (only
//     `cli_framework::app::AppBuilder` / the prelude), so the import is split.
//   - `secrecy::Secret<String>` has no `From<&str>`, only `From<String>`, so
//     every `headers: Some("...".into())` literal becomes
//     `Some("...".to_string().into())`.
// Per C1's closing instruction, the test names and assertions are otherwise
// unchanged from the plan; only the construction is fixed.
use cli_framework::app::AppBuilder;
use cli_framework::{Deployment, Identity, TelemetryDefaults};

mod support;
use support::EnvGuard;

/// Read a `SecretString` back in a test. If the crate's secret type exposes
/// this differently, use its own accessor — do not add a public one.
fn expose(s: &cli_framework::SecretString) -> String {
    use secrecy::ExposeSecret;
    s.expose_secret().to_string()
}

#[test]
fn an_app_that_configures_nothing_is_an_end_user_app_with_telemetry_off() {
    // Pin the precondition and take the same process-wide lock the
    // env-setting tests below hold, so none of them can run alongside
    // this one and make the assertion below about their variable.
    let _g = EnvGuard::unset("OTEL_EXPORTER_OTLP_ENDPOINT");
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .build_for_test();
    assert!(matches!(
        app.deployment(),
        Deployment::EndUser { privacy_url: None }
    ));
    assert_eq!(app.telemetry_policy().level.as_str(), "off");
    assert!(app.telemetry_policy().endpoint.is_none());
}

#[test]
fn a_standalone_api_server_defaults_to_a_service_deployment() {
    use cli_framework::api::ApiServerBuilder;
    assert!(matches!(
        ApiServerBuilder::new().deployment(),
        cli_framework::Deployment::Service
    ));
}

#[test]
fn a_service_with_an_endpoint_defaults_to_diagnostic_and_without_one_to_off() {
    // Pin the precondition and take the same process-wide lock the
    // env-setting tests below hold, so none of them can run alongside
    // this one and make the assertion below about their variable.
    let _g = EnvGuard::unset("OTEL_EXPORTER_OTLP_ENDPOINT");
    let with = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::Service)
        .with_telemetry_defaults(TelemetryDefaults {
            endpoint: Some("http://collector:4318".into()),
            ..Default::default()
        })
        .build_for_test();
    assert_eq!(with.telemetry_policy().level.as_str(), "diagnostic");

    let without = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::Service)
        .build_for_test();
    assert_eq!(
        without.telemetry_policy().level.as_str(),
        "off",
        "a service with nowhere to send is a service that should not pay to build spans"
    );
}

#[test]
fn an_author_supplied_header_never_appears_in_a_debug_rendering_of_the_builder() {
    let defaults = TelemetryDefaults {
        endpoint: Some("http://collector:4318".into()),
        headers: Some("authorization=Bearer sk-live-secret".to_string().into()),
        ..Default::default()
    };
    let rendered = format!("{defaults:?}");
    assert!(
        !rendered.contains("sk-live-secret"),
        "an OTLP header is usually a bearer token, and the moment someone \
         debugs their telemetry setup is exactly when it would be printed: {rendered}"
    );
}

#[test]
fn the_environment_endpoint_wins_over_the_authors_default() {
    let _g = EnvGuard::set("OTEL_EXPORTER_OTLP_ENDPOINT", "http://operator:4318");
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::Service)
        .with_telemetry_defaults(TelemetryDefaults {
            endpoint: Some("http://author-default:4318".into()),
            ..Default::default()
        })
        .build_for_test();
    assert_eq!(
        app.telemetry_policy().endpoint.as_deref(),
        Some("http://operator:4318"),
        "the author picks a default; the operator running the process decides"
    );
}

#[test]
fn the_environment_headers_win_over_the_authors_default_and_reach_the_exporter() {
    let _g = EnvGuard::set(
        "OTEL_EXPORTER_OTLP_HEADERS",
        "authorization=Bearer env-token",
    );
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::Service)
        .with_telemetry_defaults(TelemetryDefaults {
            endpoint: Some("http://collector:4318".into()),
            headers: Some("authorization=Bearer author-token".to_string().into()),
            ..Default::default()
        })
        .build_for_test();
    assert_eq!(
        app.telemetry_policy().headers.as_ref().map(expose),
        Some("authorization=Bearer env-token".to_string())
    );
}

#[test]
fn headers_are_not_a_config_field_and_appear_in_no_status_output() {
    let _g = EnvGuard::set(
        "OTEL_EXPORTER_OTLP_HEADERS",
        "authorization=Bearer env-token",
    );
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::Service)
        .with_telemetry_defaults(TelemetryDefaults {
            endpoint: Some("http://collector:4318".into()),
            ..Default::default()
        })
        .build_for_test();
    assert!(
        app.config_manifest()
            .leaf_by_path("telemetry.headers")
            .is_none(),
        "headers are off the config tree by design: a manifest field is \
         readable, roamable and printable, and this one is a bearer token"
    );
    let rendered = format!("{:?}", app.telemetry_policy());
    assert!(!rendered.contains("env-token"), "{rendered}");
}

#[test]
fn a_privacy_url_travels_on_the_deployment_and_reaches_the_notice() {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::EndUser {
            privacy_url: Some("https://example.com/privacy".into()),
        })
        .build_for_test();
    match app.deployment() {
        Deployment::EndUser { privacy_url } => {
            assert_eq!(privacy_url.as_deref(), Some("https://example.com/privacy"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn registering_an_operational_probe_puts_it_in_the_registry_and_the_manifest() {
    static OPS: &[cli_framework::telemetry::ProbeSpec] = &[cli_framework::telemetry::ProbeSpec {
        id: "app.sync",
        min_level: cli_framework::telemetry::TelemetryLevel::Usage,
        summary: "a sync ran",
        sends: "the outcome",
    }];
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_ops(OPS)
        .build_for_test();
    assert!(app.telemetry_policy().registry.get("app.sync").is_some());
    assert!(
        app.config_manifest()
            .leaf_by_path("telemetry.app.sync.enabled")
            .is_some(),
        "an author's probe is switchable by the same mechanism as a built-in one"
    );
}

#[test]
fn the_identity_resolver_is_a_closure_because_identity_is_not_known_at_build_time() {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_identity(std::sync::Arc::new(|_ctx| {
            Some(Identity {
                enduser_id: Some("u1".into()),
                tenant: Some("t1".into()),
            })
        }))
        .build_for_test();
    assert!(
        app.telemetry_identity_resolver().is_some(),
        "identity is resolved after authentication, which is after the builder \
         runs; a value would force every author to build the app twice"
    );
}

#[test]
fn an_author_attribute_allowlist_and_never_list_both_reach_the_policy() {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_attrs(vec!["app.tenant_kind".into()])
        .with_telemetry_never(vec!["employee_id".into()])
        .build_for_test();
    let policy = app.telemetry_policy();
    assert!(policy
        .app_attr_allowlist
        .contains(&"app.tenant_kind".to_string()));
    assert!(policy.extra_never.contains(&"employee_id".to_string()));
}

#[test]
fn an_author_cannot_allowlist_a_never_listed_key_back_into_existence() {
    use cli_framework::telemetry::RedactionRules;
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_attrs(vec!["app.api_key".into()])
        .build_for_test();
    let rules = RedactionRules::from_policy(app.telemetry_policy());
    assert!(
        !rules.keeps_attribute("app.api_key"),
        "the never-list is checked first and wins over the author's allowlist"
    );
}

#[test]
fn an_invalid_probe_id_in_with_telemetry_ops_fails_the_build_rather_than_being_dropped() {
    static BAD: &[cli_framework::telemetry::ProbeSpec] = &[cli_framework::telemetry::ProbeSpec {
        id: "App Sync",
        min_level: cli_framework::telemetry::TelemetryLevel::Usage,
        summary: "x",
        sends: "y",
    }];
    let result = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_ops(BAD)
        .try_build_for_test();
    assert!(
        result.is_err(),
        "silently dropping the probe would leave the author's instrumentation \
         emitting under an id nothing recognises, which the export boundary drops"
    );
}

#[test]
fn an_empty_endpoint_variable_is_not_an_endpoint() {
    // An operator who exports the variable but leaves it blank has nowhere to
    // send to. Reading that as "an endpoint exists" would flip a service from
    // `off` to `diagnostic` and then fail every export.
    let _g = EnvGuard::set("OTEL_EXPORTER_OTLP_ENDPOINT", "");
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::Service)
        .build_for_test();
    assert!(app.telemetry_policy().endpoint.is_none());
    assert_eq!(app.telemetry_policy().level.as_str(), "off");
}

#[test]
fn an_empty_headers_variable_leaves_the_authors_headers_alone() {
    let _g = EnvGuard::set("OTEL_EXPORTER_OTLP_HEADERS", "");
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::Service)
        .with_telemetry_defaults(TelemetryDefaults {
            endpoint: Some("http://collector:4318".into()),
            headers: Some("authorization=Bearer author-token".to_string().into()),
            ..Default::default()
        })
        .build_for_test();
    assert_eq!(
        app.telemetry_policy().headers.as_ref().map(expose),
        Some("authorization=Bearer author-token".to_string()),
        "an exported-but-blank variable is not an override, and treating it as \
         one would silently drop the author's authentication"
    );
}

#[test]
fn a_kill_switch_in_the_environment_beats_an_endpoint_the_author_configured() {
    let _g = EnvGuard::set("DO_NOT_TRACK", "1");
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_deployment(Deployment::Service)
        .with_telemetry_defaults(TelemetryDefaults {
            endpoint: Some("http://collector:4318".into()),
            ..Default::default()
        })
        .build_for_test();
    let policy = app.telemetry_policy();
    assert_eq!(policy.level.as_str(), "off");
    assert!(policy.kill_switch.is_some());
    assert!(
        !policy.exports(),
        "a kill switch that still built an exporter would be a kill switch in name only"
    );
}

#[test]
fn the_telemetry_file_follows_the_apps_own_configuration_format() {
    use cli_framework::config::{ConfigFormat, ConfigOptions};

    let builder = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_config(ConfigOptions::<Demo>::new(1).with_format(ConfigFormat::Toml));
    assert_eq!(
        builder.config_format(),
        ConfigFormat::Toml,
        "a TOML app must not get one lone JSON file in an otherwise-TOML directory"
    );
    assert_eq!(
        AppBuilder::new()
            .with_version("demo", "0.0.0")
            .config_format(),
        ConfigFormat::Json,
        "an app that declares no configuration still needs somewhere to keep consent"
    );
}

#[test]
fn registering_the_same_probe_id_twice_fails_the_build() {
    static ONCE: &[cli_framework::telemetry::ProbeSpec] = &[cli_framework::telemetry::ProbeSpec {
        id: "app.sync",
        min_level: cli_framework::telemetry::TelemetryLevel::Usage,
        summary: "a sync ran",
        sends: "the outcome",
    }];
    assert!(
        AppBuilder::new()
            .with_version("demo", "0.0.0")
            .with_telemetry_ops(ONCE)
            .with_telemetry_ops(ONCE)
            .try_build_for_test()
            .is_err(),
        "two registrations of one id would put two switches on one probe in \
         the manifest, and only one of them would work"
    );
}

#[test]
fn the_published_manifest_keeps_the_apps_own_fields_beside_the_telemetry_section() {
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_config_manifest(app_manifest("theme"))
        .build_for_test();
    let published = app.config_manifest();
    assert!(
        published.leaf_by_path("theme").is_some(),
        "merging the framework's section must not drop the app's own fields"
    );
    assert!(published.leaf_by_path("telemetry.level").is_some());
}

#[test]
fn an_app_that_owns_a_top_level_telemetry_key_fails_the_build() {
    assert!(
        AppBuilder::new()
            .with_version("demo", "0.0.0")
            .with_config_manifest(app_manifest("telemetry"))
            .try_build_for_test()
            .is_err(),
        "silently shadowing the app's key would leave two meanings for one \
         path and an administrator no way to tell which one they set"
    );
}

#[test]
fn a_probe_id_that_collides_with_a_reserved_first_segment_is_rejected() {
    static RESERVED: &[cli_framework::telemetry::ProbeSpec] =
        &[cli_framework::telemetry::ProbeSpec {
            id: "level.something",
            min_level: cli_framework::telemetry::TelemetryLevel::Usage,
            summary: "x",
            sends: "y",
        }];
    assert!(AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_telemetry_ops(RESERVED)
        .try_build_for_test()
        .is_err());
}

/// A minimal typed configuration, so `with_config` has something to register.
/// Hand-written rather than derived on purpose: this binary's
/// `required-features` are `telemetry` and `config`, not `derive`.
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
struct Demo {
    schema_version: u32,
}

impl cli_framework::config::VersionedConfig for Demo {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

/// An application manifest holding exactly one string field, named `key`.
fn app_manifest(key: &str) -> cli_framework::config::manifest::ConfigManifest {
    use cli_framework::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
    ConfigManifest::new(
        "demo",
        vec![FieldManifest {
            key: key.to_string(),
            kind: FieldKind::Str,
            default: None,
            label: None,
            description: None,
            group: None,
            scope: Scope::User,
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
