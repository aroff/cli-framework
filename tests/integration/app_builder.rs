//! Integration tests for the built-in `version` command.

use std::sync::OnceLock;
use std::sync::{Arc, Mutex};

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::CommandArgs;
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::{CommandPath, CommandSpec, GroupMetadata};

#[cfg(feature = "testkit")]
use cli_framework::app::AppMeta;
#[cfg(feature = "testkit")]
use cli_framework::testkit::CliTestHarness;

#[path = "../stdio_capture.rs"]
mod stdio_capture;
use stdio_capture::strip_test_harness_noise;
use stdio_capture::StderrCapture;
use stdio_capture::StdoutCapture;

struct DummyCtx;
impl AppContext for DummyCtx {}

struct LogCollector {
    records: Arc<Mutex<Vec<String>>>,
}

impl log::Log for LogCollector {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            self.records
                .lock()
                .unwrap()
                .push(format!("{}", record.args()));
        }
    }
    fn flush(&self) {}
}

static TEST_LOG_RECORDS: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();

fn install_test_logger() -> Arc<Mutex<Vec<String>>> {
    let records = TEST_LOG_RECORDS.get_or_init(|| {
        let records: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let collector = LogCollector {
            records: records.clone(),
        };
        let _ = log::set_boxed_logger(Box::new(collector));
        log::set_max_level(log::LevelFilter::Warn);
        records
    });
    records.clone()
}

#[tokio::test]
async fn version_dispatch_with_version_configured() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();

    // Acquire stdio_lock via StdoutCapture to prevent the version string written
    // to fd 1 from racing with concurrent StdoutCapture-based tests (e.g. completion
    // stub tests that dup2-redirect fd 1 to a temp file).  The capture itself is
    // discarded; the assertion below uses app.version_string() instead.
    let cap = StdoutCapture::new();
    app.run_with_args(vec!["myapp".to_string(), "version".to_string()])
        .await
        .unwrap();
    let _ = cap.finish();

    assert_eq!(app.version_string(), "myapp 1.2.3");
}

#[tokio::test]
async fn version_dispatch_double_dash_version() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec!["myapp".to_string(), "--version".to_string()])
        .await
        .unwrap();
    let _ = cap.finish();

    assert_eq!(app.version_string(), "myapp 1.2.3");
}

#[tokio::test]
async fn version_dispatch_without_with_version_prints_unknown() {
    let mut app = AppBuilder::new().build(DummyCtx).unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec!["myapp".to_string(), "version".to_string()])
        .await
        .unwrap();
    let _ = cap.finish();

    assert_eq!(app.version_string(), "unknown unknown");
}

#[tokio::test]
async fn execute_command_version_returns_not_found() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();

    let result = app.execute_command("version", CommandArgs::default()).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn show_help_contains_version_entry() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();

    let help = app.render_help();
    assert!(help.contains("version"));

    let version_line = help.lines().next().unwrap();
    assert_eq!(version_line, "  version - Print version information");
}

#[test]
#[cfg(not(feature = "strict-types"))]
fn show_help_version_appears_before_registered_commands() {
    use cli_framework::command::Command;

    let cmd = Command {
        id: "alpha",
        summary: "Alpha command",
        syntax: None,
        category: Some("test"),
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
    };

    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .register_command(cmd)
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let help = app.render_help();
    let version_pos = help.find("version - Print version information").unwrap();
    let alpha_pos = help.find("alpha").unwrap();
    assert!(version_pos < alpha_pos);
}

#[test]
fn warn_log_emitted_when_version_not_configured() {
    let records = install_test_logger();
    records.lock().unwrap().clear();

    let mut app = AppBuilder::new().build(DummyCtx).unwrap();

    let cap = StdoutCapture::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(app.run_with_args(vec!["myapp".to_string(), "version".to_string()]))
        .unwrap();
    let _output = cap.finish();

    let msgs = records.lock().unwrap();
    assert!(msgs
        .iter()
        .any(|m| m.contains("with_version() was not configured")));
}

#[tokio::test]
#[cfg(feature = "testkit")]
async fn version_and_flags_include_git_sha_when_configured() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .with_git_sha_short(Some("abc1234"))
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);

    let out = harness.run(&["myapp", "--version"]).await;
    out.assert_exit_code(0);
    assert_eq!(out.stdout, "myapp 1.2.3 (abc1234)\n");

    let out = harness.run(&["myapp", "-V"]).await;
    out.assert_exit_code(0);
    assert_eq!(out.stdout, "myapp 1.2.3 (abc1234)\n");

    let out = harness.run(&["myapp", "version"]).await;
    out.assert_exit_code(0);
    assert_eq!(out.stdout, "myapp 1.2.3 (abc1234)\n");
}

#[tokio::test]
#[cfg(feature = "testkit")]
async fn version_flags_omit_git_sha_when_unset() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .with_git_sha_short(None)
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["myapp", "--version"]).await;
    out.assert_exit_code(0);
    let line = out.stdout.trim_end();
    assert!(line.contains("myapp 1.2.3"));
    assert!(!line.contains('('));
    assert!(!line.contains(')'));
}

#[tokio::test]
#[cfg(feature = "testkit")]
async fn invalid_git_sha_is_omitted_and_warns() {
    let records = install_test_logger();
    records.lock().unwrap().clear();

    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .with_git_sha_short(Some("not-a-sha"))
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);
    let out = harness.run(&["myapp", "version"]).await;
    out.assert_exit_code(0);
    assert_eq!(out.stdout.trim_end(), "myapp 1.2.3");

    let msgs = records.lock().unwrap();
    assert!(msgs.iter().any(|m| m.contains("ERR_VERSION_SHA_001")));
}

#[tokio::test]
#[cfg(feature = "testkit")]
async fn meta_overrides_name_and_version_consistently_for_version_output() {
    let app = AppBuilder::new()
        .with_version("builder-name", "0.0.1")
        .with_meta(AppMeta {
            name: "meta-name",
            version: "9.9.9",
            description: "desc",
            usage: None,
        })
        .with_git_sha_short(Some("abc1234"))
        .build(DummyCtx)
        .unwrap();

    let mut harness = CliTestHarness::new(app);

    let out = harness.run(&["meta-name", "--version"]).await;
    out.assert_exit_code(0);
    assert_eq!(out.stdout.trim_end(), "meta-name 9.9.9 (abc1234)");

    let out = harness.run(&["meta-name", "version"]).await;
    out.assert_exit_code(0);
    assert_eq!(out.stdout.trim_end(), "meta-name 9.9.9 (abc1234)");
}

#[cfg(feature = "clap-dispatch")]
mod clap_dispatch_tests {
    use super::*;

    fn hello_command() -> cli_framework::command::Command {
        cli_framework::command::Command {
            id: "hello",
            summary: "Say hello",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
        }
    }

    #[tokio::test]
    #[cfg(not(feature = "strict-types"))]
    async fn clap_help_shows_subcommands() {
        let app = AppBuilder::new()
            .with_version("myapp", "1.2.3")
            .register_command(hello_command())
            .unwrap()
            .build(DummyCtx)
            .unwrap();

        let cap = StdoutCapture::new();
        let mut app = app;
        app.run_with_args(vec!["myapp".to_string(), "--help".to_string()])
            .await
            .unwrap();
        let output = cap.finish();

        assert!(output.contains("hello"));
        assert!(output.contains("version"));
    }

    #[tokio::test]
    async fn clap_unknown_command_returns_usage_error() {
        let mut app = AppBuilder::new()
            .with_version("myapp", "1.2.3")
            .build(DummyCtx)
            .unwrap();

        let result = app
            .run_with_args(vec!["myapp".to_string(), "bogus".to_string()])
            .await;
        // R1: unrecognized subcommand must return Err(UsageError), not Ok.
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<cli_framework::UsageError>()
                .is_some(),
            "expected UsageError for unknown command"
        );
    }

    #[tokio::test]
    #[cfg(not(feature = "strict-types"))]
    async fn clap_key_equals_value_parsing() {
        use std::sync::Mutex;

        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured.clone();

        let cmd = cli_framework::command::Command {
            id: "hello",
            summary: "Say hello",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: Arc::new(move |_ctx, args| {
                let captured = captured_clone.clone();
                Box::pin(async move {
                    let name = args.named.get("name").cloned().unwrap_or_default();
                    captured.lock().unwrap().push(name);
                    Ok(())
                })
            }),
        };

        let mut app = AppBuilder::new()
            .with_version("myapp", "1.2.3")
            .register_command(cmd)
            .unwrap()
            .build(DummyCtx)
            .unwrap();

        app.run_with_args(vec![
            "myapp".to_string(),
            "hello".to_string(),
            "--name=Alice".to_string(),
        ])
        .await
        .unwrap();

        let vals = captured.lock().unwrap();
        assert_eq!(vals[0], "Alice");
    }

    #[tokio::test]
    async fn clap_no_args_shows_help() {
        let app = AppBuilder::new()
            .with_version("myapp", "1.2.3")
            .build(DummyCtx)
            .unwrap();

        let mut app = app;
        let cap = StdoutCapture::new();
        let result = app.run_with_args(vec!["myapp".to_string()]).await;
        let _ = cap.finish();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn clap_dash_h_shows_help() {
        let app = AppBuilder::new()
            .with_version("myapp", "1.2.3")
            .build(DummyCtx)
            .unwrap();

        let cap = StdoutCapture::new();
        let mut app = app;
        app.run_with_args(vec!["myapp".to_string(), "-h".to_string()])
            .await
            .unwrap();
        let output = cap.finish();
        assert!(!output.is_empty());
    }

    #[test]
    fn clap_render_help_preserves_custom_format() {
        let app = AppBuilder::new()
            .with_version("myapp", "1.2.3")
            .build(DummyCtx)
            .unwrap();

        let help = app.render_help();
        assert!(help.contains("version - Print version information"));
        assert!(help.contains("Options:"));
    }

    // AC-G2.2: `prog --version` outputs "{name} {version}" format.
    #[tokio::test]
    async fn clap_version_flag_output_format() {
        let mut app = AppBuilder::new()
            .with_version("testapp", "3.5.7")
            .build(DummyCtx)
            .unwrap();

        let cap = StdoutCapture::new();
        app.run_with_args(vec!["testapp".to_string(), "--version".to_string()])
            .await
            .unwrap();
        let output = cap.finish();

        let trimmed = output.trim();
        assert!(
            trimmed.contains("testapp"),
            "expected version output to contain app name, got: {:?}",
            trimmed
        );
        assert!(
            trimmed.contains("3.5.7"),
            "expected version output to contain version, got: {:?}",
            trimmed
        );
    }

    // AC-G5.3: `prog unknown_cmd` produces stderr containing "unrecognized subcommand".
    #[cfg(feature = "testkit")]
    #[tokio::test]
    async fn clap_unknown_command_stderr_contains_unrecognized() {
        let app = AppBuilder::new()
            .with_version("myapp", "1.0.0")
            .build(DummyCtx)
            .unwrap();
        let mut harness = CliTestHarness::new(app);

        let out = harness.run(&["myapp", "bogus"]).await;
        // R1 + R5: unrecognized subcommand must exit 2 (usage error).
        out.assert_exit_code(2);
        out.assert_stderr_contains("unrecognized");
    }

    // AC-G5.4: `prog hello --nonexistent-flag` behavior.
    //
    // **Known limitation:** With the `trailing_var_arg` approach (necessary for
    // dynamic commands whose flags are unknown at build time), Clap captures
    // `--nonexistent-flag` as a trailing string rather than rejecting it as an
    // unknown flag. This is a documented deviation from the spec. The test
    // verifies that the command still executes successfully and the unknown
    // flag is available in the args for the command to handle.
    #[tokio::test]
    #[cfg(not(feature = "strict-types"))]
    async fn clap_unknown_flag_captured_silently() {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured.clone();

        let cmd = cli_framework::command::Command {
            id: "hello",
            summary: "Say hello",
            syntax: None,
            category: None,
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: Arc::new(move |_ctx, args| {
                let captured = captured_clone.clone();
                Box::pin(async move {
                    let all_args: Vec<String> = args
                        .named
                        .values()
                        .chain(args.positional.iter())
                        .cloned()
                        .collect();
                    *captured.lock().unwrap() = all_args;
                    Ok(())
                })
            }),
        };

        let mut app = AppBuilder::new()
            .with_version("myapp", "1.0.0")
            .register_command(cmd)
            .unwrap()
            .build(DummyCtx)
            .unwrap();

        let result = app
            .run_with_args(vec![
                "myapp".to_string(),
                "hello".to_string(),
                "--nonexistent-flag".to_string(),
            ])
            .await;

        assert!(
            result.is_ok(),
            "with trailing_var_arg, unknown flags are captured, not rejected"
        );

        // The bare --nonexistent-flag is not inserted as "true" per DD#8,
        // so captured args should be empty (flag is skipped, not stored).
        let vals = captured.lock().unwrap();
        assert!(
            vals.is_empty(),
            "bare --flag without value should not appear in named or positional (DD#8)"
        );
    }
}

fn make_hidden_cmd(id: &'static str, hidden: bool) -> cli_framework::command::Command {
    cli_framework::command::Command {
        id,
        summary: "test",
        syntax: None,
        category: None,
        spec: Some(Arc::new(CommandSpec {
            summary: "test",
            hidden,
            args: vec![],
            ..Default::default()
        })),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
    }
}

#[tokio::test]
async fn completion_bash_stub_shape_and_candidates_are_sorted_and_filtered() {
    let hidden = make_hidden_cmd("hidden_cmd", true);
    let visible = make_hidden_cmd("alpha", false);

    let mut app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .register_command(visible)
        .unwrap()
        .register_command(hidden)
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec![
        "myapp".to_string(),
        "completion".to_string(),
        "bash".to_string(),
    ])
    .await
    .unwrap();
    let out = cap.finish();
    // When stdout is globally redirected via dup2, the Rust test harness may write
    // progress markers and per-test status lines (from other parallel tests) into
    // the same stream. Filter that noise out before asserting.
    let out = strip_test_harness_noise(&out);
    let out = out.as_str();

    let first_non_blank = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    assert!(
        first_non_blank.starts_with("_myapp()") || first_non_blank.starts_with("complete "),
        "unexpected first non-blank line: {:?}",
        first_non_blank
    );

    assert!(
        out.contains("compgen -W \""),
        "expected bash stub to contain compgen candidate list; got:\n{}",
        out
    );
    let candidates = out
        .lines()
        .find_map(|l| {
            let start = l.find("compgen -W \"")?;
            let rest = &l[start + "compgen -W \"".len()..];
            let end = rest.find("\" -- ")?;
            Some(rest[..end].to_string())
        })
        .unwrap_or_default();
    let parsed: Vec<&str> = candidates.split_whitespace().collect();
    let mut sorted = parsed.clone();
    sorted.sort();
    assert_eq!(
        parsed, sorted,
        "expected deterministic sorted output, got: {:?}",
        parsed
    );
    for expected in ["alpha", "completion", "spec"] {
        assert!(
            parsed.contains(&expected),
            "expected candidate {:?} in {:?}",
            expected,
            parsed
        );
    }
    assert!(!out.contains("hidden_cmd"));
}

#[tokio::test]
async fn completion_zsh_stub_starts_with_compdef() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .build(DummyCtx)
        .unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec![
        "myapp".to_string(),
        "completion".to_string(),
        "zsh".to_string(),
    ])
    .await
    .unwrap();
    let out = cap.finish();
    let out = strip_test_harness_noise(&out);

    let first_non_blank = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    assert_eq!(first_non_blank, "#compdef myapp");
}

#[tokio::test]
async fn completion_fish_stub_contains_one_line_per_candidate() {
    let extra = make_hidden_cmd("extra", false);
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .register_command(extra)
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec![
        "myapp".to_string(),
        "completion".to_string(),
        "fish".to_string(),
    ])
    .await
    .unwrap();
    let out = cap.finish();

    assert!(out.contains("complete -c myapp -f"));
    for cmd in ["completion", "extra", "spec"] {
        assert!(
            out.contains(&format!(
                "complete -c myapp -n '__fish_use_subcommand' -a '{}'",
                cmd
            )),
            "missing fish completion line for {}:\n{}",
            cmd,
            out
        );
    }
}

#[tokio::test]
async fn completion_powershell_stub_contains_register_argument_completer() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .build(DummyCtx)
        .unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec![
        "myapp".to_string(),
        "completion".to_string(),
        "powershell".to_string(),
    ])
    .await
    .unwrap();
    let out = cap.finish();

    assert!(out.contains("Register-ArgumentCompleter -Native -CommandName myapp"));
}

#[tokio::test]
async fn completions_hidden_alias_routes_but_does_not_appear_in_help() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .build(DummyCtx)
        .unwrap();

    // Alias routes
    let cap = StdoutCapture::new();
    app.run_with_args(vec![
        "myapp".to_string(),
        "completions".to_string(),
        "bash".to_string(),
    ])
    .await
    .unwrap();
    let out = cap.finish();
    assert!(out.contains("complete -F _myapp myapp"));

    // Alias hidden from help
    let cap = StdoutCapture::new();
    app.run_with_args(vec!["myapp".to_string(), "--help".to_string()])
        .await
        .unwrap();
    let help = cap.finish();
    assert!(help.contains("completion"));
    assert!(!help.contains("completions"));
}

#[tokio::test]
async fn completion_invalid_shell_emits_single_diagnostic_and_returns_usage_error() {
    // R3 + R4a: invalid shell value is rejected at parse time (Enum validation → E004).
    // The error is emitted exactly once (no double-print from E013 path).
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .build(DummyCtx)
        .unwrap();

    let stderr_cap = StderrCapture::new();
    let result = app
        .run_with_args(vec![
            "myapp".to_string(),
            "completion".to_string(),
            "invalidshell".to_string(),
        ])
        .await;
    let stderr = stderr_cap.finish();

    // R1/R5: must return Err(UsageError).
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.downcast_ref::<cli_framework::UsageError>().is_some(),
        "expected UsageError, got: {}",
        err
    );

    // R3: exactly one diagnostic line, mentions the invalid value.
    let diag_lines: Vec<&str> = stderr.lines().filter(|l| l.contains("error[")).collect();
    assert_eq!(
        diag_lines.len(),
        1,
        "expected exactly one error diagnostic line, got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("invalidshell"),
        "diagnostic should name the invalid value, got:\n{}",
        stderr
    );
}

#[tokio::test]
async fn without_completion_disables_builtin_and_allows_user_completion() {
    let user_completion = cli_framework::command::Command {
        id: "completion",
        summary: "user completion",
        syntax: None,
        category: None,
        spec: Some(Arc::new(CommandSpec {
            summary: "user completion",
            args: vec![ArgSpec {
                name: "shell",
                kind: ArgKind::Positional,
                short: None,
                long: None,
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "shell",
            }],
            ..Default::default()
        })),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|ctx, _args| {
            Box::pin(async move {
                ctx.framework_println("user");
                Ok(())
            })
        }),
    };

    let mut app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .without_completion()
        .register_command(user_completion)
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let cap = StdoutCapture::new();
    app.run_with_args(vec!["myapp".to_string(), "completion".to_string()])
        .await
        .unwrap();
    let out = cap.finish();
    assert!(out.contains("user"));
}

#[tokio::test]
async fn completion_includes_root_segment_from_visible_leaf_even_when_group_hidden() {
    let mut builder = AppBuilder::new().with_version("myapp", "1.0.0");
    builder = builder
        .register_group(
            &CommandPath::root_for("grp"),
            GroupMetadata {
                summary: "grp",
                hidden: true,
            },
        )
        .unwrap();

    let grp_leaf_path = CommandPath::new(&["grp", "show"]).unwrap();
    builder = builder
        .register_command_at(&grp_leaf_path, make_hidden_cmd("show", false))
        .unwrap();

    let mut app = builder.build(DummyCtx).unwrap();
    let cap = StdoutCapture::new();
    app.run_with_args(vec![
        "myapp".to_string(),
        "completion".to_string(),
        "bash".to_string(),
    ])
    .await
    .unwrap();
    let out = cap.finish();
    let candidates = out
        .lines()
        .find_map(|l| {
            let start = l.find("compgen -W \"")?;
            let rest = &l[start + "compgen -W \"".len()..];
            let end = rest.find("\" -- ")?;
            Some(rest[..end].to_string())
        })
        .unwrap_or_default();
    let parsed: Vec<&str> = candidates.split_whitespace().collect();
    assert!(parsed.contains(&"grp"), "expected 'grp' in {:?}", parsed);
}

// ============================================================================
// Testkit-based versions of version dispatch tests (Stage 7 migration)
// ============================================================================

#[cfg(feature = "testkit")]
mod testkit_version_tests {
    use super::*;

    #[tokio::test]
    async fn version_dispatch_with_version_configured_testkit() {
        let app = AppBuilder::new()
            .with_version("myapp", "1.2.3")
            .build(DummyCtx)
            .unwrap();

        let mut harness = CliTestHarness::new(app);
        let output = harness.run(&["myapp", "version"]).await;

        assert_eq!(output.stdout(), "myapp 1.2.3\n");
        assert_eq!(output.exit_code(), 0);
    }

    #[tokio::test]
    async fn version_dispatch_double_dash_version_testkit() {
        let app = AppBuilder::new()
            .with_version("myapp", "1.2.3")
            .build(DummyCtx)
            .unwrap();

        let mut harness = CliTestHarness::new(app);
        let output = harness.run(&["myapp", "--version"]).await;

        assert_eq!(output.stdout(), "myapp 1.2.3\n");
        assert_eq!(output.exit_code(), 0);
    }

    #[tokio::test]
    async fn version_dispatch_without_with_version_prints_unknown_testkit() {
        let app = AppBuilder::new().build(DummyCtx).unwrap();

        let mut harness = CliTestHarness::new(app);
        let output = harness.run(&["myapp", "version"]).await;

        assert_eq!(output.stdout(), "unknown unknown\n");
        assert_eq!(output.exit_code(), 0);
    }
}
