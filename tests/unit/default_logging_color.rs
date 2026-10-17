//! `init_default_logging` colours its lines only where a person reads them.
//!
//! Installing the subscriber is a process-global one-way door, so each case
//! runs in a fresh copy of this test binary: the parent re-executes itself
//! with `CFW_LOG_COLOR_CHILD` set and stdout piped (not a terminal), and the
//! child installs the subscriber and logs one line.

use std::process::Command;

const CHILD: &str = "CFW_LOG_COLOR_CHILD";
const LINE: &str = "default logging colour probe";

/// The child half: a no-op unless the parent started this process.
#[test]
fn child_logs_one_line() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let _guard = cli_framework::init_default_logging();
    tracing::info!("{LINE}");
}

fn child_stdout(env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "child_logs_one_line",
        "--nocapture",
        "--test-threads=1",
    ])
    .env(CHILD, "1")
    .env_remove("NO_COLOR")
    .env_remove("FORCE_COLOR")
    .env_remove("RUST_LOG");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run the child");
    assert!(out.status.success(), "child failed: {out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains(LINE), "the child logged nothing:\n{stdout}");
    stdout
}

#[test]
fn a_pipe_gets_plain_lines() {
    let stdout = child_stdout(&[]);
    assert!(
        !stdout.contains('\x1b'),
        "escape codes in a pipe:\n{stdout:?}"
    );
}

#[test]
fn force_color_colours_a_pipe() {
    let stdout = child_stdout(&[("FORCE_COLOR", "1")]);
    assert!(stdout.contains('\x1b'), "FORCE_COLOR ignored:\n{stdout:?}");
}

#[test]
fn no_color_wins_over_force_color() {
    let stdout = child_stdout(&[("FORCE_COLOR", "1"), ("NO_COLOR", "1")]);
    assert!(!stdout.contains('\x1b'), "NO_COLOR ignored:\n{stdout:?}");
}
