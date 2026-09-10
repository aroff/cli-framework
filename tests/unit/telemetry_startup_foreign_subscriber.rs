// tests/unit/telemetry_startup_foreign_subscriber.rs
//
//! One test, one process: an application that installed its own subscriber
//! before the framework got there. Installing a subscriber is a one-way door,
//! so this cannot share a binary with `unit_telemetry_startup_wiring` — same
//! reason PR4 Task 14 split `unit_telemetry_subscriber_foreign` out.

use cli_framework::config::resolution::Layer;
use cli_framework::config::ConfigFormat;
use cli_framework::telemetry::{
    run_startup, telemetry_only_manifest, Deployment, ProbeRegistry, ServiceIdentity,
    StartupInputs, SubscriberOutcome, Surface, TelemetryInputs, TelemetryStoreLocation,
};

#[test]
fn an_application_that_installed_its_own_subscriber_keeps_it_and_still_gets_metrics() {
    tracing_subscriber::fmt()
        .with_writer(std::io::sink)
        .try_init()
        .expect("this test owns the process global");

    let dir = tempfile::tempdir().expect("a temp dir");
    let registry = ProbeRegistry::with_builtins();
    let manifest = telemetry_only_manifest("demo", &registry, None);
    let inputs = StartupInputs {
        base: TelemetryInputs {
            app: "demo".to_string(),
            deployment: Deployment::Service,
            endpoint: Some("http://127.0.0.1:9/".to_string()),
            endpoint_source: Some(Layer::Default),
            session_id: "session-fixture".to_string(),
            registry,
            ..Default::default()
        },
        store: TelemetryStoreLocation {
            dir: Some(dir.path().to_path_buf()),
            format: ConfigFormat::Json,
        },
        manifest: std::sync::Arc::new(manifest),
        service: ServiceIdentity {
            name: "demo".to_string(),
            version: "0.0.0".to_string(),
        },
        surface: Surface::Cli,
        stderr_is_tty: false,
        env: Vec::new(),
    };

    let result = run_startup(inputs);

    assert!(
        result.policy.exports(),
        "the fixture must export, or there is no subscriber to lose"
    );
    assert_eq!(
        result.report.subscriber,
        SubscriberOutcome::ForeignSubscriber,
        "startup must not steal a subscriber the application installed"
    );
    assert!(
        result
            .report
            .findings
            .iter()
            .any(|f| f.check_id == "telemetry.subscriber"),
        "a degraded install has to be visible in `doctor`, or the missing \
         traces look like a collector problem"
    );
    assert!(
        result.guard.is_some(),
        "losing the subscriber costs traces, not metrics — the meter provider \
         does not go through `tracing` at all"
    );

    tracing::info!("the application's own subscriber still works");
}
