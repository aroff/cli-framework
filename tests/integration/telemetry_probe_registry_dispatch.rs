//! Proves the real dispatch wiring for `AppContext::opt_probe_registry`,
//! not just the hand-rolled test double that exercises the trait's default
//! method contract.
//!
//! `tests/unit/telemetry_probe_feature.rs` drives `AppContext::mark_feature`
//! through `FeatureTestCtx`, a context that hands the trait its own
//! `ProbeRegistry` by constructing one directly. That never touches
//! `CliAppContextWrapper::opt_probe_registry` (`src/app/dispatch.rs`) — the
//! override every command handler actually sees when dispatched through a
//! real `AppBuilder`, which hands back `self.env.probe_registry` (in turn
//! `&self.telemetry_policy.registry`, `src/app/builder.rs`). Nothing in the
//! existing suite calls that override at all, so it carried zero coverage.
//!
//! This is deliberately not a `mark_feature`/`FeatureOutcome::Recorded` test:
//! there is currently no public `AppBuilder` API that adds a name to the
//! registry before it reaches dispatch. `ProbeRegistry::register_feature`
//! (`src/telemetry/probe.rs`) is the real mechanism, but no builder method
//! exposes it — `src/app/context.rs` and `src/telemetry/probe.rs` both
//! point applications at `AppBuilder::with_telemetry_ops` for this, and
//! that method does not exist anywhere in the crate (confirmed by grep). So
//! a `Recorded` outcome is not reachable through a real, external
//! `AppBuilder`-driven application today. What this test proves instead —
//! the only thing about this wiring that a real dispatch *can* prove right
//! now — is that the handler-visible registry is `Some` and is the
//! framework's real one (carrying the built-in `cli.command` probe), not
//! `None` or an empty stand-in.
//!
//! # Why this calls `.with_telemetry`
//!
//! An earlier version of this test deliberately did *not* call
//! `.with_telemetry`, on the theory that the registry accessor and the
//! export pipeline were independent concerns. They are not, currently:
//! `AppBuilder::build()` seeds `telemetry_policy` from
//! `TelemetryInputs::default()` (`src/app/builder.rs`), whose `registry`
//! field is a plain-derived `ProbeRegistry::default()` — empty. Only
//! `init_telemetry()`'s `if let Some(ref cfg) = self.telemetry_config`
//! branch replaces it with one built via `ProbeRegistry::with_builtins()`,
//! and that branch runs only when `.with_telemetry(cfg)` was called before
//! `.build()`. An app that never calls `.with_telemetry()` therefore gets a
//! *present but empty* registry: `opt_probe_registry()` still returns
//! `Some(_)`, but `.contains("cli.command")` on it is `false`. (Confirmed
//! empirically: this file, before this fix, called no `.with_telemetry` and
//! asserted `Some(true)`, and observed `Some(false)`.)
//!
//! Whether a zero-config app *should* still see the built-in probes is a
//! policy-orchestration question — the same kind `src/app/builder.rs`'s
//! `telemetry_policy` field doc defers to PR7 for the redaction-boundary
//! gap (see `tests/integration/telemetry_end_to_end.rs`). It is not decided
//! here. What this file tests is the mainline case any application that
//! actually wants telemetry will hit: register commands, call
//! `.with_telemetry(cfg)`, dispatch. The mock server below exists only so
//! `init_batch`'s exporter construction and the guard's drop-time flush
//! have somewhere to POST; this test asserts nothing about what lands
//! there.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::Command;
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::telemetry::TelemetryConfig;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct ProbeCtx;
impl AppContext for ProbeCtx {}

/// Reads `ctx.opt_probe_registry()` from inside a real command handler and
/// records whether it was populated with the built-in `cli.command` probe.
fn registry_reading_command(seen: Arc<Mutex<Option<bool>>>) -> Command {
    Command {
        id: Arc::from("read-registry"),
        spec: Arc::new(CommandSpec {
            summary: "Reads the probe registry the dispatcher hands the context",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(move |ctx, _args| {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                let has_builtin_command_probe = ctx
                    .opt_probe_registry()
                    .map(|registry| registry.contains("cli.command"))
                    .unwrap_or(false);
                *seen.lock().unwrap() = Some(has_builtin_command_probe);
                Ok(())
            })
        }),
    }
}

#[tokio::test]
async fn a_real_dispatch_hands_the_command_context_the_builtin_probe_registry() {
    let seen = Arc::new(Mutex::new(None));

    let server = MockServer::start().await;
    for p in ["/v1/traces", "/v1/metrics"] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b""))
            .mount(&server)
            .await;
    }

    let cfg = TelemetryConfig {
        endpoint: Some(server.uri()),
        ..Default::default()
    };

    // See the module doc: only the `.with_telemetry(cfg)` branch of
    // `AppBuilder::init_telemetry` populates `telemetry_policy.registry`
    // with `ProbeRegistry::with_builtins()`. Omitting this call is not a
    // simplification — it silently switches the assertion below onto the
    // empty-registry path and fails for an unrelated reason.
    let mut app = AppBuilder::new()
        .with_version("probeapp", "1.0.0")
        .register_command(registry_reading_command(Arc::clone(&seen)))
        .unwrap()
        .with_telemetry(cfg)
        .build(ProbeCtx)
        .unwrap();

    // Keep any command output out of libtest's stdout.
    app.stdout_capture = Some(Arc::new(Mutex::new(Vec::new())));

    app.run_with_args(vec!["probeapp".to_string(), "read-registry".to_string()])
        .await
        .expect("dispatch of a trivial registered command should not error");

    assert_eq!(
        *seen.lock().unwrap(),
        Some(true),
        "CliAppContextWrapper::opt_probe_registry did not hand the command \
         handler a registry containing the built-in cli.command probe — it \
         returned None, or an empty/wrong registry. AppBuilder wires this \
         from telemetry_policy.registry (src/app/builder.rs) through \
         dispatch's probe_registry field (src/app/dispatch.rs); either link \
         breaking would surface here."
    );
}
