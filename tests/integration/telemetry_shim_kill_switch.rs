//! A kill switch stops the deprecated `with_telemetry` shim exporting.
//!
//! Spec 025 puts the three kill switches — `<APP>_TELEMETRY_DISABLED=1`,
//! `OTEL_SDK_DISABLED=true`, `DO_NOT_TRACK=1` — ahead of every other layer.
//! `OTEL_SDK_DISABLED` already stopped the shim, through
//! `TelemetryConfig::is_active`. The other two are *new*, so honouring them
//! on the shim's path takes nothing away from a deployment that works today
//! — nobody sets a variable the framework has never read — while a shim that
//! defeated them would be a hole in the switch it exists to obey.
//!
//! The oracle here is a collector, not a policy field: the policy said "off"
//! long before this fix, and the shim exported anyway. Only what arrives at
//! `MockServer` can tell those two apart.
//!
//! # Why this is one test in its own binary
//!
//! The same reason `telemetry_end_to_end.rs` is: telemetry installs a
//! *process-global* subscriber, and `DO_NOT_TRACK` is a *process-global*
//! environment variable. A second test in this binary would race both.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::Command;
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::telemetry::TelemetryConfig;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct ProbeCtx;
impl AppContext for ProbeCtx {}

fn probe_command() -> Command {
    Command {
        id: Arc::from("probe"),
        spec: Arc::new(CommandSpec {
            summary: "Probe command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
    }
}

const TRACES: &str = "/v1/traces";
const METRICS: &str = "/v1/metrics";

// Exercises the deprecated `with_telemetry` shim on purpose: it has to keep
// obeying a kill switch until it is removed in v0.8.0.
#[allow(deprecated)]
#[tokio::test]
async fn do_not_track_stops_the_deprecated_shim_exporting() {
    let server = MockServer::start().await;
    for p in [TRACES, METRICS] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b""))
            .mount(&server)
            .await;
    }

    // Set before `build`, because the build-time resolution reads the
    // environment too. Removed at the end of the test; this binary holds one
    // test precisely so nothing else can observe it in between.
    std::env::set_var("DO_NOT_TRACK", "1");

    // The spec 025 sequence is what runs once the shim is stopped, and it
    // opens the framework-owned settings file. Point it somewhere disposable
    // rather than at the developer's real configuration directory.
    let store = tempfile::tempdir().expect("tempdir");

    let cfg = TelemetryConfig {
        endpoint: Some(server.uri()),
        ..Default::default()
    };

    let mut app = AppBuilder::new()
        .with_version("probeapp", "1.2.3")
        .register_command(probe_command())
        .unwrap()
        .with_telemetry(cfg)
        .with_telemetry_config_dir(store.path())
        .build(ProbeCtx)
        .unwrap();

    app.stdout_capture = Some(Arc::new(Mutex::new(Vec::new())));

    app.run_with_args(vec!["probeapp".to_string(), "probe".to_string()])
        .await
        .expect("CLI dispatch panicked or errored under a Tokio runtime");

    // The same beat `telemetry_end_to_end` waits: long enough for the batch
    // worker to land a POST if one was ever queued. Without it this test
    // would pass by being fast rather than by being right.
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;

    let requests = server.received_requests().await.unwrap_or_default();
    let hits: Vec<String> = requests.iter().map(|r| r.url.path().to_string()).collect();

    std::env::remove_var("DO_NOT_TRACK");

    assert!(
        hits.is_empty(),
        "DO_NOT_TRACK=1 was set and the app still exported to the collector: \
         {hits:?}. A kill switch that a deprecated shim can defeat is not a \
         kill switch"
    );

    // Why the export stopped, not merely that it did: with the shim's own
    // pipeline skipped, the spec 025 startup sequence is what ran instead,
    // and it records what it observed. A run that exported nothing because
    // the shim silently failed to start would leave this `None`.
    let report = app
        .startup_report()
        .expect("the spec 025 startup sequence did not run in place of the shim");
    assert_eq!(
        report.kill_switch,
        Some(cli_framework::telemetry::KillSwitch::DoNotTrack),
        "startup ran but did not attribute the shutdown to DO_NOT_TRACK"
    );
}
