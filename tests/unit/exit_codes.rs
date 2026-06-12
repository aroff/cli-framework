//! Tests for spec 012: usage-error exit codes and argument validation.
//!
//! Contract (R5):
//!   - Usage / parse errors → `Err(UsageError)` from `run_with_args`; `App::run()` exits 2.
//!   - Runtime errors       → `Err(<non-UsageError>)` from `run_with_args`; exits 1.
//!   - Success              → `Ok(())`, exits 0.

use cli_framework::app::{AppBuilder, AppContext, UsageError};
use cli_framework::command::Command;
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::sync::Arc;

struct DummyCtx;
impl AppContext for DummyCtx {}

#[allow(clippy::type_complexity)]
fn noop_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn AppContext,
            HashMap<String, ArgValue>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>,
        > + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

fn typed_cmd(id: &'static str, args: Vec<ArgSpec>) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary: id,
            args,
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }
}

// ── R1: Non-zero exit for unrecognized subcommands ───────────────────────────

#[tokio::test]
async fn r1_unknown_command_returns_usage_error() {
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();
    let result = app
        .run_with_args(vec!["testapp".to_string(), "totallybogus".to_string()])
        .await;

    assert!(result.is_err(), "unknown command must return Err");
    assert!(
        result.unwrap_err().downcast_ref::<UsageError>().is_some(),
        "unknown command must be UsageError (exit 2)"
    );
}

/// R1 + R5: testkit maps UsageError → exit code 2.
/// Uses testkit but avoids checking stderr content to stay race-free.
#[tokio::test]
async fn r1_unknown_command_testkit_exit_code_2() {
    use cli_framework::testkit::CliTestHarness;

    let app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();
    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["testapp", "totallybogus"]).await;
    out.assert_exit_code(2);
}

// ── R2: Missing required argument emits E003, not E001 ───────────────────────

#[tokio::test]
async fn r2_known_command_missing_required_arg_returns_e003_parse_error() {
    use cli_framework::app::clap_adapter::{build_clap_root, parse_with_clap};
    use cli_framework::command::CommandRegistry;
    use cli_framework::parser::outcome::ParseOutcome;
    use cli_framework::spec::command_tree::CommandPath;

    let mut registry = CommandRegistry::new();
    registry
        .register_at(
            &CommandPath::root_for("review"),
            typed_cmd(
                "review",
                vec![ArgSpec {
                    name: "template",
                    kind: ArgKind::Option,
                    short: Some('t'),
                    long: None,
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Required,
                    default: None,
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Template name",
                    ..Default::default()
                }],
            ),
        )
        .unwrap();

    let root = build_clap_root(None, &registry, "testapp", "0.1.0", None, &[]);
    // Invoke `review` without the required `--template` arg.
    let outcome = parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "review".to_string()],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(
                d.code,
                cli_framework::parser::error_codes::E_MISSING_REQUIRED,
                "expected E003, got {}: {}",
                d.code,
                d.message
            );
            // Must NOT say "unrecognized subcommand" — that was the R2 bug.
            assert!(
                !d.message.contains("unrecognized"),
                "E003 message must not say 'unrecognized', got: {}",
                d.message
            );
        }
        other => panic!("expected ParseError(E003), got {:?}", other),
    }
}

/// R2: completion without <shell> → UsageError (not "unrecognized subcommand").
#[tokio::test]
async fn r2_completion_without_shell_is_usage_error_not_e001() {
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec!["testapp".to_string(), "completion".to_string()])
        .await;

    assert!(result.is_err(), "completion without shell must return Err");
    let err = result.unwrap_err();
    // Must be a UsageError (exit 2)
    assert!(
        err.downcast_ref::<UsageError>().is_some(),
        "completion without shell must be UsageError, got: {}",
        err
    );
    // Message must not say "unrecognized subcommand" (that was the R2 bug)
    assert!(
        !err.to_string().contains("unrecognized"),
        "error must not say 'unrecognized', got: {}",
        err
    );
}

// ── R3 + R4a: completion with invalid shell → single diagnostic, exit 2 ─────
// Verified via parse-layer (no testkit) to avoid global capture race.

#[tokio::test]
async fn r3_r4a_completion_invalid_shell_is_usage_error_single_parse_error() {
    use cli_framework::app::clap_adapter::{build_clap_root, parse_with_clap};
    use cli_framework::parser::outcome::ParseOutcome;

    // Build registry that has `completion` registered (same as App::build does).
    let full_app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();
    let registry = full_app.command_registry().clone();

    let root = build_clap_root(None, &registry, "testapp", "0.1.0", None, &[]);
    let outcome = parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "completion".to_string(),
            "zzz".to_string(),
        ],
        &[],
        true,
    );

    // R4a: invalid Enum value must be caught at parse time as a single E004 error.
    match outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(
                d.code,
                cli_framework::parser::error_codes::E_INVALID_VALUE,
                "expected E004, got {}: {}",
                d.code,
                d.message
            );
            // R3: message names the invalid value
            assert!(
                d.message.contains("zzz"),
                "diagnostic must name the invalid value, got: {}",
                d.message
            );
            // R3: message lists allowed shells (spec 011 §2: bash | zsh | fish | powershell | pwsh)
            assert!(
                d.message.contains("bash") && d.message.contains("pwsh"),
                "diagnostic must list allowed shells including pwsh, got: {}",
                d.message
            );
        }
        other => panic!(
            "expected ParseError(E004) for invalid shell, got {:?}",
            other
        ),
    }
}

/// R3: run_with_args with invalid shell returns a single UsageError (exit 2).
#[tokio::test]
async fn r3_completion_invalid_shell_returns_usage_error() {
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "completion".to_string(),
            "zzz".to_string(),
        ])
        .await;

    assert!(result.is_err());
    assert!(
        result.unwrap_err().downcast_ref::<UsageError>().is_some(),
        "invalid shell must be UsageError (exit 2)"
    );
}

// ── R4a: Enum validation rejects invalid values at parse time ────────────────

#[tokio::test]
async fn r4a_enum_invalid_value_rejected_as_usage_error() {
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .register_command(typed_cmd(
            "review",
            vec![ArgSpec {
                name: "focus",
                kind: ArgKind::Option,
                short: None,
                long: None,
                value_type: ArgValueType::Enum(vec!["security", "performance", "style"]),
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Review focus area",
                ..Default::default()
            }],
        ))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "review".to_string(),
            "--focus".to_string(),
            "bogus_value".to_string(),
        ])
        .await;

    // R4a: must fail before handler runs, as UsageError (exit 2)
    assert!(result.is_err(), "invalid Enum value must return Err");
    let err = result.unwrap_err();
    assert!(
        err.downcast_ref::<UsageError>().is_some(),
        "invalid Enum value must be UsageError (exit 2), got: {}",
        err
    );
    // Error message must name the invalid value
    assert!(
        err.to_string().contains("bogus_value"),
        "error must name the invalid value, got: {}",
        err
    );
}

/// R4a: the E004 parse error message lists allowed values.
#[tokio::test]
async fn r4a_enum_invalid_value_error_lists_allowed() {
    use cli_framework::app::clap_adapter::{build_clap_root, parse_with_clap};
    use cli_framework::command::CommandRegistry;
    use cli_framework::parser::outcome::ParseOutcome;
    use cli_framework::spec::command_tree::CommandPath;

    let mut registry = CommandRegistry::new();
    registry
        .register_at(
            &CommandPath::root_for("review"),
            typed_cmd(
                "review",
                vec![ArgSpec {
                    name: "focus",
                    kind: ArgKind::Option,
                    short: None,
                    long: None,
                    value_type: ArgValueType::Enum(vec!["security", "performance"]),
                    cardinality: Cardinality::Optional,
                    default: None,
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "",
                    ..Default::default()
                }],
            ),
        )
        .unwrap();

    let root = build_clap_root(None, &registry, "testapp", "0.1.0", None, &[]);
    let outcome = parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "review".to_string(),
            "--focus".to_string(),
            "zzz".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(d.code, cli_framework::parser::error_codes::E_INVALID_VALUE);
            assert!(
                d.message.contains("zzz"),
                "must name the invalid value, got: {}",
                d.message
            );
            assert!(
                d.message.contains("security") && d.message.contains("performance"),
                "must list allowed values, got: {}",
                d.message
            );
        }
        other => panic!("expected ParseError(E004), got {:?}", other),
    }
}

// ── R4a: valid Enum value passes through correctly ───────────────────────────

#[tokio::test]
async fn r4a_enum_valid_value_passes_to_handler() {
    use std::sync::Mutex;

    let received: Arc<Mutex<Option<ArgValue>>> = Arc::new(Mutex::new(None));
    let received_clone = received.clone();

    let cmd = Command {
        id: Arc::from("review"),
        spec: Arc::new(CommandSpec {
            summary: "review",
            args: vec![ArgSpec {
                name: "focus",
                kind: ArgKind::Option,
                short: None,
                long: None,
                value_type: ArgValueType::Enum(vec!["security", "performance"]),
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "",
                ..Default::default()
            }],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(move |_ctx, args| {
            let val = args.get("focus").cloned();
            let r = received_clone.clone();
            Box::pin(async move {
                *r.lock().unwrap() = val;
                Ok(())
            })
        }),
    };

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .register_command(cmd)
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "review".to_string(),
            "--focus".to_string(),
            "security".to_string(),
        ])
        .await;

    assert!(
        result.is_ok(),
        "valid Enum value must succeed: {:?}",
        result
    );
    assert_eq!(
        received.lock().unwrap().as_ref(),
        Some(&ArgValue::Enum("security".to_string())),
        "handler must receive the valid Enum value"
    );
}

// ── R4b: Repeated arg delivers all occurrences to handler ────────────────────

#[tokio::test]
async fn r4b_repeated_enum_arg_delivers_all_values() {
    use std::sync::Mutex;

    let received: Arc<Mutex<Vec<ArgValue>>> = Arc::new(Mutex::new(Vec::new()));
    let received_clone = received.clone();

    let cmd = Command {
        id: Arc::from("review"),
        spec: Arc::new(CommandSpec {
            summary: "review",
            args: vec![ArgSpec {
                name: "focus",
                kind: ArgKind::Option,
                short: None,
                long: None,
                value_type: ArgValueType::Enum(vec!["security", "performance", "style"]),
                cardinality: Cardinality::Repeated,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "",
                ..Default::default()
            }],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(move |_ctx, args| {
            let vals = match args.get("focus") {
                Some(ArgValue::List(v)) => v.clone(),
                _ => vec![],
            };
            let r = received_clone.clone();
            Box::pin(async move {
                *r.lock().unwrap() = vals;
                Ok(())
            })
        }),
    };

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .register_command(cmd)
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "review".to_string(),
            "--focus".to_string(),
            "security".to_string(),
            "--focus".to_string(),
            "performance".to_string(),
        ])
        .await;

    assert!(result.is_ok(), "repeated Enum must succeed: {:?}", result);
    let vals = received.lock().unwrap().clone();
    assert_eq!(
        vals.len(),
        2,
        "handler must receive all 2 occurrences, got: {:?}",
        vals
    );
    assert_eq!(vals[0], ArgValue::Enum("security".to_string()));
    assert_eq!(vals[1], ArgValue::Enum("performance".to_string()));
}

// ── R4b: Repeated arg with invalid value is rejected ─────────────────────────

#[tokio::test]
async fn r4b_repeated_enum_invalid_value_rejected() {
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .register_command(typed_cmd(
            "review",
            vec![ArgSpec {
                name: "focus",
                kind: ArgKind::Option,
                short: None,
                long: None,
                value_type: ArgValueType::Enum(vec!["security", "performance"]),
                cardinality: Cardinality::Repeated,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "",
                ..Default::default()
            }],
        ))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "review".to_string(),
            "--focus".to_string(),
            "security".to_string(),
            "--focus".to_string(),
            "zzz".to_string(), // invalid second value
        ])
        .await;

    assert!(
        result.is_err(),
        "invalid Enum value in repeated arg must fail"
    );
    assert!(
        result.unwrap_err().downcast_ref::<UsageError>().is_some(),
        "must be UsageError"
    );
}

// ── R5: Exit-code contract — runtime errors are NOT UsageError ───────────────

#[tokio::test]
async fn r5_runtime_error_is_not_usage_error() {
    let cmd = Command {
        id: Arc::from("failing"),
        spec: Arc::new(CommandSpec {
            summary: "always fails",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| {
            Box::pin(async { Err(anyhow::anyhow!("runtime failure")) })
        }),
    };

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .register_command(cmd)
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec!["testapp".to_string(), "failing".to_string()])
        .await;

    assert!(result.is_err(), "failing command must return Err");
    assert!(
        result.unwrap_err().downcast_ref::<UsageError>().is_none(),
        "runtime error must NOT be UsageError (must exit 1, not 2)"
    );
}

// ── R5: validation failures (E005 conflict) are UsageError ──────────────────

#[tokio::test]
async fn r5_validation_conflict_is_usage_error() {
    let cmd = typed_cmd(
        "cmd",
        vec![
            ArgSpec {
                name: "a",
                kind: ArgKind::Flag,
                short: None,
                long: None,
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec!["b"],
                requires: vec![],
                help: "",
                ..Default::default()
            },
            ArgSpec {
                name: "b",
                kind: ArgKind::Flag,
                short: None,
                long: None,
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "",
                ..Default::default()
            },
        ],
    );

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .register_command(cmd)
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    // Pass both conflicting flags
    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "cmd".to_string(),
            "--a".to_string(),
            "--b".to_string(),
        ])
        .await;

    // R5: validation failure (E005) must be UsageError (exit 2)
    assert!(result.is_err(), "conflict validation must fail");
    assert!(
        result.unwrap_err().downcast_ref::<UsageError>().is_some(),
        "validation failure (E005) must be UsageError (exit 2)"
    );
}

// ── DR003: unknown doctor check is a UsageError ──────────────────────────────

#[cfg(feature = "doctor")]
#[tokio::test]
async fn dr003_unknown_check_is_usage_error() {
    use cli_framework::doctor::check::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};

    struct OkCheck;
    impl DoctorCheck for OkCheck {
        fn id(&self) -> &'static str {
            "ok-check"
        }
        fn title(&self) -> &'static str {
            "OK"
        }
        fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
            Box::pin(async {
                DoctorFinding {
                    check_id: "ok-check".to_string(),
                    title: "OK".to_string(),
                    severity: CheckSeverity::Ok,
                    message: "all good".to_string(),
                    detail: None,
                    remediation: None,
                }
            })
        }
    }

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .register_doctor_checks(vec![Arc::new(OkCheck)])
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "doctor".to_string(),
            "--check".to_string(),
            "nonexistent-check".to_string(),
        ])
        .await;

    // DR003 + R5: unknown check id must be a UsageError (exit 2)
    assert!(result.is_err(), "unknown check must return Err");
    assert!(
        result.unwrap_err().downcast_ref::<UsageError>().is_some(),
        "DR003 (unknown check id) must be UsageError (exit 2)"
    );
}
