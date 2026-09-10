// tests/unit/telemetry_startup_wiring.rs
//
//! Startup wiring: the ten steps of `startup_order()`, actually executed.
//!
//! # Why the inputs are a value and not the builder
//!
//! The plan sketched `run_startup(builder: &AppBuilder)`. That signature
//! cannot serve the caller that matters. `App::run_with_args` holds an
//! `App`, not an `AppBuilder` — the builder is consumed by `build()` long
//! before the first command runs — so a `&AppBuilder` parameter would force
//! startup to happen at *build* time. Every one of the ~180 `build(Ctx)`
//! call sites in this test suite would then open the real framework store in
//! the developer's own configuration directory.
//!
//! `StartupInputs` is the fix: `AppBuilder::build` records what startup will
//! need, `App::startup_inputs()` hands it over, and `run_startup` is a pure
//! function of that value. The process environment arrives the same way — as
//! a snapshot in `StartupInputs::env`, never read from `std::env` inside the
//! function. That is why there is no `EnvGuard` anywhere in this file and no
//! cross-test environment race is possible: a test that wants
//! `DEMO_TELEMETRY_LEVEL=debug` sets it in a `Vec`, not in the process.

use cli_framework::config::manifest::ConfigManifest;
use cli_framework::config::resolution::Layer;
use cli_framework::config::ConfigFormat;
use cli_framework::telemetry::{
    run_startup, run_startup_recording, startup_order, telemetry_only_manifest, Attribution,
    Deployment, ProbeRegistry, ServiceIdentity, StartupInputs, StartupStep, Surface,
    TelemetryInputs, TelemetryLevel, TelemetryStore, TelemetryStoreLocation, END_USER_FLUSH_BUDGET,
};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// `init_from_policy` writes OpenTelemetry's process-global tracer and meter
/// providers. Two exporting tests running concurrently would each stomp on
/// the other's provider, and a guard dropped by one would shut down a
/// provider the other still holds. Same rationale as
/// `telemetry_pipeline.rs`'s lock of the same name; taken with
/// `unwrap_or_else(|e| e.into_inner())` so one panicking test does not turn
/// every later test in this binary into a poison failure.
fn otel_global_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// A loopback port nothing listens on. Refused immediately, where the
/// fixtures' usual `http://collector:4318` has to fail DNS resolution first
/// and drags a multi-second timeout into every exporting test.
const DEAD_COLLECTOR: &str = "http://127.0.0.1:9/";

/// Startup inputs for an app whose telemetry file lives under `dir`.
///
/// Deliberately builds `TelemetryInputs` by hand rather than going through
/// `AppBuilder`: all but one test here is about what `run_startup` does with
/// its inputs, and reaching through the builder would make every one of them
/// depend on the builder's own defaults as well.
fn inputs_at(dir: &Path) -> StartupInputs {
    let registry = ProbeRegistry::with_builtins();
    let manifest = telemetry_only_manifest("demo", &registry, None);
    StartupInputs {
        base: TelemetryInputs {
            app: "demo".to_string(),
            deployment: Deployment::EndUser { privacy_url: None },
            attribution: Attribution::Pseudonymous,
            session_id: "session-fixture".to_string(),
            registry,
            ..Default::default()
        },
        store: TelemetryStoreLocation {
            dir: Some(dir.to_path_buf()),
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
    }
}

/// Turn a fixture into one that actually exports, so the provider,
/// subscriber and panic-hook steps genuinely run.
fn exporting(mut inputs: StartupInputs, deployment: Deployment) -> StartupInputs {
    inputs.base.deployment = deployment;
    inputs.base.endpoint = Some(DEAD_COLLECTOR.to_string());
    inputs.base.endpoint_source = Some(Layer::Default);
    inputs
}

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn position(recorded: &[StartupStep], step: StartupStep) -> usize {
    recorded
        .iter()
        .position(|s| *s == step)
        .unwrap_or_else(|| panic!("startup never ran {step:?}; it ran {recorded:?}"))
}

// ---------------------------------------------------------------------------
// The order itself
// ---------------------------------------------------------------------------

/// The point of `startup_order()` is that it describes what the code does.
/// An implementation that imports the constant and then does something else
/// compiles fine and passes any test that only checks the constant, so this
/// asserts against what `run_startup` *recorded while running*.
///
/// The fixture is a `Service` deployment with an endpoint precisely so every
/// step is reached: a policy that does not export builds no providers, holds
/// no subscriber layer and installs no panic hook, and would make the
/// comparison below pass against a five-step implementation.
#[test]
fn startup_executes_the_steps_in_the_pinned_order() {
    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("a temp dir");
    let inputs = exporting(inputs_at(dir.path()), Deployment::Service);

    let (result, recorded) = run_startup_recording(inputs);

    assert!(
        result.policy.exports(),
        "the fixture must export or the provider steps never run and this \
         test passes for the wrong reason"
    );
    assert_eq!(
        recorded,
        startup_order().to_vec(),
        "startup ran a different order than the one it publishes"
    );
}

// ---------------------------------------------------------------------------
// Kill switches
// ---------------------------------------------------------------------------

/// `DO_NOT_TRACK=1` must cost nothing: no disk access, no socket. The
/// assertion is not just that the step list is short — it is that the
/// telemetry file was never created on disk.
#[test]
fn a_kill_switch_stops_startup_before_the_store_is_ever_opened() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut inputs = inputs_at(dir.path());
    inputs.env = env(&[("DO_NOT_TRACK", "1")]);

    let (result, recorded) = run_startup_recording(inputs);

    assert!(
        result.policy.kill_switch.is_some(),
        "the snapshot in `StartupInputs::env` must be what detects the kill \
         switch; reading `std::env` instead would find nothing here"
    );
    assert_eq!(result.policy.level, TelemetryLevel::Off);
    for skipped in [
        StartupStep::OpenStore,
        StartupStep::MergeManifest,
        StartupStep::BuildProviders,
        StartupStep::InstallSubscriber,
        StartupStep::InstallPanicHook,
    ] {
        assert!(
            !recorded.contains(&skipped),
            "a kill switch must skip {skipped:?}, but startup ran {recorded:?}"
        );
    }
    assert!(
        !dir.path().join("demo").exists(),
        "the store directory was created despite a kill switch, so \
         `DO_NOT_TRACK=1` still touched the disk"
    );
    assert!(
        recorded.contains(&StartupStep::Dispatch),
        "a kill switch disables telemetry, not the application"
    );
    assert!(
        result.guard.is_none(),
        "a kill switch must build no provider"
    );
    assert!(result.handle.is_none());
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// A store failure is a value, never an abort. The fixture points the store
/// directory *inside a regular file*, so `create_dir_all` fails with
/// `ENOTDIR` for every user on every platform — a read-only parent directory
/// would not do, because root sails straight through one.
#[test]
fn an_unwritable_store_does_not_stop_startup() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"regular file").expect("the blocker file writes");

    let inputs = inputs_at(&blocker.join("nested"));
    let (result, recorded) = run_startup_recording(inputs);

    assert!(
        !result.report.store.is_ready(),
        "the fixture must actually be unwritable, or this test proves nothing"
    );
    assert!(
        result.report.store.reason().is_some(),
        "an unavailable store must carry the reason, for the doctor check"
    );
    assert!(
        !result.policy.store_available,
        "the policy must know the store failed"
    );
    assert!(
        recorded.contains(&StartupStep::Dispatch),
        "an unwritable store must not stop the application from running"
    );
}

/// PRD 025: with no store there is no install id to be pseudonymous with, so
/// attribution degrades rather than inventing one per process.
#[test]
fn an_unavailable_store_degrades_attribution_to_anonymous() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"regular file").expect("the blocker file writes");

    let result = run_startup(inputs_at(&blocker.join("nested")));

    assert_eq!(result.policy.attribution, Attribution::Anonymous);
    assert!(
        result.policy.install_id.is_none(),
        "an anonymous Install must carry no install id"
    );
}

// ---------------------------------------------------------------------------
// The frozen policy
// ---------------------------------------------------------------------------

/// "Frozen into an `Arc` and never mutated after this point" is only a real
/// guarantee if the providers hold *that* `Arc`. A second resolution handed
/// to the exporter would satisfy any ordering assertion while letting the
/// export boundary filter against a different policy than the one reported.
#[test]
fn the_policy_is_frozen_before_any_provider_is_built() {
    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("a temp dir");
    let inputs = exporting(inputs_at(dir.path()), Deployment::Service);

    let (result, recorded) = run_startup_recording(inputs);

    assert!(
        position(&recorded, StartupStep::FreezePolicy)
            < position(&recorded, StartupStep::BuildProviders),
        "the policy must be frozen before a provider can capture it"
    );
    assert!(
        std::sync::Arc::strong_count(&result.policy) > 1,
        "the export boundary is holding a different policy than the one \
         startup froze, so redaction is filtering against the wrong rules"
    );
}

// ---------------------------------------------------------------------------
// The panic hook
// ---------------------------------------------------------------------------

#[test]
fn the_panic_hook_is_installed_before_the_first_command_can_panic() {
    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("a temp dir");
    let inputs = exporting(inputs_at(dir.path()), Deployment::Service);

    let (result, recorded) = run_startup_recording(inputs);

    assert!(
        result.panic_hook,
        "an exporting policy must install the `cli.panic` hook"
    );
    assert!(
        position(&recorded, StartupStep::InstallPanicHook)
            < position(&recorded, StartupStep::Dispatch),
        "a hook installed after dispatch cannot report the command's panic"
    );
}

// ---------------------------------------------------------------------------
// The notice
// ---------------------------------------------------------------------------

#[test]
fn the_notice_is_shown_before_dispatch_so_it_is_not_buried_under_command_output() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut inputs = inputs_at(dir.path());
    inputs.stderr_is_tty = true;

    let (result, recorded) = run_startup_recording(inputs);

    let notice = result
        .notice
        .as_deref()
        .expect("a first run on a terminal must show the notice");
    assert!(
        notice.contains("demo telemetry set usage"),
        "the notice must tell the reader how to turn telemetry on; got {notice:?}"
    );
    assert!(
        position(&recorded, StartupStep::ShowNotice) < position(&recorded, StartupStep::Dispatch),
        "a notice printed after the command's own output is buried"
    );
}

/// PRD 025: after printing, the announced level is stored, so the second run
/// is silent. The assertion reads the store back rather than trusting the
/// second call, so an implementation that forgets to persist fails here
/// rather than accidentally passing on a shared in-memory value.
#[test]
fn the_notice_is_persisted_so_the_second_run_is_silent() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut first = inputs_at(dir.path());
    first.stderr_is_tty = true;
    assert!(
        run_startup(first).notice.is_some(),
        "the first run must show the notice or the second proves nothing"
    );

    let stored = TelemetryStore::open_at_with_format(dir.path(), "demo", ConfigFormat::Json)
        .settings()
        .notice_shown;
    assert_eq!(
        stored,
        Some(TelemetryLevel::Off),
        "the announced level must be written to the telemetry file"
    );

    let mut second = inputs_at(dir.path());
    second.stderr_is_tty = true;
    assert!(
        run_startup(second).notice.is_none(),
        "the notice must not repeat on every run"
    );
}

// ---------------------------------------------------------------------------
// The fold: stored settings and the environment layer
// ---------------------------------------------------------------------------

/// The stored `telemetry.level` is consent. Build-time resolution cannot see
/// it — the file is read at startup — so this is the step that makes an
/// opted-in user's choice take effect.
#[test]
fn startup_folds_the_stored_consent_into_the_policy() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = TelemetryStore::open_at_with_format(dir.path(), "demo", ConfigFormat::Json);
    store
        .mutate(|s| s.level = Some(TelemetryLevel::Usage))
        .expect("the fixture store is writable");

    let result = run_startup(inputs_at(dir.path()));

    assert_eq!(result.policy.level, TelemetryLevel::Usage);
    assert_eq!(
        result.policy.level_source,
        Layer::ConfigFile,
        "consent comes from the stored file, and `telemetry status` reports \
         that provenance to the person who gave it"
    );
    assert!(
        result.policy.install_id.is_some(),
        "a pseudonymous Install with a working store must carry an install id"
    );
}

#[test]
fn startup_folds_the_environment_layer_and_records_unmatched_variables() {
    // This fixture exports, and `init_from_policy` sets the OpenTelemetry
    // process globals unconditionally, so it has to serialise with every
    // other exporting test in this binary.
    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("a temp dir");
    // `Service`, because the end-user clamp exists precisely to stop the
    // environment raising the telemetry level on an Install.
    let mut inputs = exporting(inputs_at(dir.path()), Deployment::Service);
    inputs.env = env(&[
        ("DEMO_TELEMETRY_LEVEL", "debug"),
        ("DEMO_TELEMETRY_NOT_A_LEAF", "1"),
    ]);

    let result = run_startup(inputs);

    assert_eq!(result.policy.level, TelemetryLevel::Debug);
    assert_eq!(result.policy.level_source, Layer::Environment);
    assert_eq!(
        result.report.unmatched_env,
        vec!["DEMO_TELEMETRY_NOT_A_LEAF".to_string()],
        "a misspelled `<APP>_TELEMETRY_*` variable must be reported, not \
         silently ignored"
    );
}

/// The clamp is `effective = min(full resolution, resolution without
/// environment)`. An Install that stored `usage` and is then handed
/// `DEMO_TELEMETRY_LEVEL=debug` stays at `usage`.
#[test]
fn the_enduser_clamp_survives_the_startup_fold() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = TelemetryStore::open_at_with_format(dir.path(), "demo", ConfigFormat::Json);
    store
        .mutate(|s| s.level = Some(TelemetryLevel::Usage))
        .expect("the fixture store is writable");

    let mut inputs = inputs_at(dir.path());
    inputs.env = env(&[("DEMO_TELEMETRY_LEVEL", "debug")]);

    let result = run_startup(inputs);

    assert_eq!(
        result.policy.level,
        TelemetryLevel::Usage,
        "the environment must not be able to raise an Install's telemetry \
         level above what its owner consented to"
    );
    assert_eq!(result.policy.level_source, Layer::ConfigFile);
}

// ---------------------------------------------------------------------------
// The flush budget
// ---------------------------------------------------------------------------

#[test]
fn an_end_user_guard_flushes_within_the_bounded_budget() {
    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut inputs = exporting(
        inputs_at(dir.path()),
        Deployment::EndUser { privacy_url: None },
    );
    // The clamp blocks the environment, not a stored choice, so consent is
    // what makes an end-user fixture export at all.
    let store = TelemetryStore::open_at_with_format(dir.path(), "demo", ConfigFormat::Json);
    store
        .mutate(|s| s.level = Some(TelemetryLevel::Usage))
        .expect("the fixture store is writable");
    inputs.base.endpoint_source = Some(Layer::Default);

    let result = run_startup(inputs);

    let guard = result
        .guard
        .as_ref()
        .expect("an end-user Install that consented and has an endpoint exports");
    assert_eq!(
        guard.flush_budget(),
        Some(END_USER_FLUSH_BUDGET),
        "a person's shell must not hang on a dead collector at exit"
    );
}

#[test]
fn a_service_deployment_guard_is_not_bounded() {
    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("a temp dir");
    let inputs = exporting(inputs_at(dir.path()), Deployment::Service);

    let result = run_startup(inputs);

    let guard = result
        .guard
        .as_ref()
        .expect("a Service with an endpoint exports");
    assert_eq!(
        guard.flush_budget(),
        None,
        "a service shutting down on SIGTERM must flush everything it has, \
         not the first 500 ms of it"
    );
}

/// The stored file carries more than consent. `attribution`, `endpoint` and
/// the per-probe switches all live in it, and each one has a different
/// observable consequence: attribution decides whether an install id exists
/// at all, the endpoint carries its own provenance so `telemetry status` can
/// name the file it came from, and a probe switch is a two-way door -- it has
/// to be able to turn a probe back *on*, not only off, or a default-off probe
/// could never be opted into.
#[test]
fn startup_folds_stored_attribution_endpoint_and_both_directions_of_a_probe_switch() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = TelemetryStore::open_at_with_format(dir.path(), "demo", ConfigFormat::Json);
    store
        .mutate(|s| {
            s.attribution = Some(Attribution::Anonymous);
            s.endpoint = Some(DEAD_COLLECTOR.to_string());
            s.probes.insert("cli.command".to_string(), true);
            s.probes.insert("cli.help".to_string(), false);
        })
        .expect("the fixture store is writable");

    let mut inputs = inputs_at(dir.path());
    // Disabled going in, so the stored `true` has something to undo. A test
    // that started from the default (enabled) would pass against a fold that
    // handled only the `false` direction.
    inputs
        .base
        .disabled_probes
        .insert("cli.command".to_string());

    let result = run_startup(inputs);

    assert_eq!(
        result.policy.attribution,
        Attribution::Anonymous,
        "the fixture asks for pseudonymous; only the stored file can have \
         changed it"
    );
    assert!(
        result.policy.install_id.is_none(),
        "an anonymous Install must carry no install id, however writable its \
         store is"
    );
    assert_eq!(
        result.policy.endpoint.as_deref(),
        Some(DEAD_COLLECTOR),
        "the stored endpoint must reach the policy"
    );
    assert_eq!(
        result.policy.endpoint_source,
        Some(Layer::ConfigFile),
        "`telemetry status` names the layer an endpoint came from; an \
         endpoint read from the file and reported as a default sends a \
         person looking in the wrong place"
    );
    assert!(
        !result.policy.disabled_probes.contains("cli.command"),
        "a stored `true` must re-enable a probe that was disabled in the \
         inputs -- the switch is a two-way door"
    );
    assert!(
        result.policy.disabled_probes.contains("cli.help"),
        "a stored `false` must disable the probe"
    );
}

/// The same three, from the environment layer, which carries its own
/// provenance (`Layer::Environment`) and is what a CI job or a container
/// actually sets.
#[test]
fn startup_folds_environment_attribution_endpoint_and_probe_switches() {
    // `Service` with an endpoint resolves to `diagnostic`, so this fixture
    // exports and `init_from_policy` writes the OpenTelemetry process
    // globals.
    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut inputs = inputs_at(dir.path());
    inputs.base.deployment = Deployment::Service;
    inputs
        .base
        .disabled_probes
        .insert("cli.command".to_string());
    inputs.env = env(&[
        ("DEMO_TELEMETRY_ATTRIBUTION", "anonymous"),
        ("DEMO_TELEMETRY_ENDPOINT", DEAD_COLLECTOR),
        ("DEMO_TELEMETRY_CLI_COMMAND_ENABLED", "1"),
        ("DEMO_TELEMETRY_CLI_HELP_ENABLED", "0"),
    ]);

    let result = run_startup(inputs);

    assert_eq!(result.policy.attribution, Attribution::Anonymous);
    assert_eq!(result.policy.endpoint.as_deref(), Some(DEAD_COLLECTOR));
    assert_eq!(
        result.policy.endpoint_source,
        Some(Layer::Environment),
        "an endpoint set by `DEMO_TELEMETRY_ENDPOINT` must be reported as \
         coming from the environment, not from the file it overrode"
    );
    assert!(
        !result.policy.disabled_probes.contains("cli.command"),
        "`DEMO_TELEMETRY_CLI_COMMAND_ENABLED=1` must re-enable the probe"
    );
    assert!(
        result.policy.disabled_probes.contains("cli.help"),
        "`DEMO_TELEMETRY_CLI_HELP_ENABLED=0` must disable the probe"
    );
    assert!(
        result.policy.level > TelemetryLevel::Off,
        "a Service with an endpoint defaults to diagnostic; without that \
         this test would not have exercised the export path at all"
    );
}

/// `install_id` and `notice_shown` are published manifest leaves, so
/// `scan_environment` matches their variables and hands them to the fold --
/// but the store owns both. An environment variable that could rewrite an
/// Install's identity would defeat `local_only`.
#[test]
fn the_environment_cannot_rewrite_the_install_id() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = TelemetryStore::open_at_with_format(dir.path(), "demo", ConfigFormat::Json);
    // Opted in, so a pseudonymous install id is minted and there is something
    // for the environment to have overwritten.
    store
        .mutate(|s| s.level = Some(TelemetryLevel::Usage))
        .expect("the fixture store is writable");

    let mut inputs = inputs_at(dir.path());
    inputs.env = env(&[("DEMO_TELEMETRY_INSTALL_ID", "identity-from-the-environment")]);

    let result = run_startup(inputs);

    let install_id = result
        .policy
        .install_id
        .as_deref()
        .expect("an opted-in pseudonymous Install with a writable store has an id");
    assert_ne!(
        install_id, "identity-from-the-environment",
        "`DEMO_TELEMETRY_INSTALL_ID` must not be able to name an Install; \
         the id is minted once, on the person's own disk"
    );
}

/// The other half of the same rule. `notice_shown` records that a *person*
/// was shown the notice; an environment variable that could pre-set it would
/// let a deployment suppress the first-run disclosure for people who had
/// never seen it.
#[test]
fn the_environment_cannot_pre_silence_the_first_run_notice() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut inputs = inputs_at(dir.path());
    inputs.stderr_is_tty = true;
    inputs.env = env(&[("DEMO_TELEMETRY_NOTICE_SHOWN", "usage")]);

    let result = run_startup(inputs);

    assert!(
        result.notice.is_some(),
        "`DEMO_TELEMETRY_NOTICE_SHOWN` must not stand in for a notice this \
         person was never shown"
    );
}

// ---------------------------------------------------------------------------
// The bridge from the builder
// ---------------------------------------------------------------------------

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

#[test]
fn startup_inputs_from_a_builder_carry_the_published_manifest_and_the_store_format() {
    use cli_framework::app::AppBuilder;
    use cli_framework::config::ConfigOptions;

    let dir = tempfile::tempdir().expect("a temp dir");
    let app = AppBuilder::new()
        .with_version("demo", "0.0.0")
        .with_config(ConfigOptions::<Demo>::new(1).with_format(ConfigFormat::Toml))
        .with_telemetry_config_dir(dir.path())
        .build_for_test();

    let inputs = app.startup_inputs();

    assert_eq!(
        inputs.store.format,
        ConfigFormat::Toml,
        "a TOML app must not get one lone JSON file in an otherwise-TOML \
         directory (PRD 025 line 258)"
    );
    assert_eq!(
        inputs.store.dir.as_deref(),
        Some(dir.path()),
        "the test store directory the builder was given must reach startup, \
         or every harness test writes to the developer's own config directory"
    );
    let manifest: &ConfigManifest = &inputs.manifest;
    assert!(
        manifest
            .iter_leaves()
            .iter()
            .any(|leaf| leaf.path == "telemetry.level"),
        "startup resolves the environment layer against the *published* \
         manifest; without the generated telemetry section it can name no \
         leaf and `DEMO_TELEMETRY_LEVEL` silently stops working"
    );
    assert_eq!(inputs.base.app, "demo");
}
