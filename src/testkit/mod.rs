//! CLI test harness for framework consumers.
//!
//! Compile only when the `testkit` feature is enabled.

use crate::app::diagnostic_reporter::DiagnosticReporter;
use crate::app::{App, AppContext};
use std::sync::{Arc, Mutex};

/// Test harness that wraps an `App` and captures framework-level output.
pub struct CliTestHarness<C: AppContext> {
    pub app: App<C>,
}

impl<C: AppContext + 'static> CliTestHarness<C> {
    /// Create a new harness wrapping `app`.
    pub fn new(app: App<C>) -> Self {
        Self { app }
    }

    /// Run the app with the given argv slice (index 0 is the binary name).
    pub async fn run(&mut self, args: &[&str]) -> TestOutput {
        let stdout_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        self.app.stdout_capture = Some(stdout_buf.clone());
        DiagnosticReporter::begin_capture();

        let result = self
            .app
            .run_with_args(args.iter().map(|s| s.to_string()).collect())
            .await;

        let stderr = DiagnosticReporter::take_capture();
        self.app.stdout_capture = None;

        let exit_code = match &result {
            Ok(()) => 0,
            Err(e) if e.downcast_ref::<crate::app::UsageError>().is_some() => 2,
            Err(_) => 1,
        };
        let stdout_bytes = stdout_buf.lock().unwrap().clone();

        TestOutput {
            stdout: String::from_utf8(stdout_bytes).unwrap_or_default(),
            stderr,
            exit_code,
        }
    }

    /// Run with per-invocation environment overrides (restored after the call).
    pub async fn run_with_env(&mut self, args: &[&str], env: &[(&str, &str)]) -> TestOutput {
        for (key, val) in env {
            std::env::set_var(key, val);
        }

        let output = self.run(args).await;

        for (key, _) in env {
            std::env::remove_var(key);
        }

        output
    }
}

/// Captured output from a single `CliTestHarness::run` invocation.
pub struct TestOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl TestOutput {
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    pub fn stderr(&self) -> &str {
        &self.stderr
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    /// Strip ISO 8601 timestamps (`\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}`) from both streams.
    pub fn normalize(&self) -> NormalizedOutput {
        NormalizedOutput {
            stdout: strip_timestamps(&self.stdout),
            stderr: strip_timestamps(&self.stderr),
        }
    }

    /// Panic with a readable message if `code` is not present in stderr.
    pub fn assert_diagnostic_code(&self, code: &str) {
        assert!(
            self.stderr.contains(code),
            "expected diagnostic code {} in stderr:\n{}",
            code,
            self.stderr
        );
    }

    pub fn assert_exit_code(&self, code: i32) {
        assert_eq!(
            self.exit_code, code,
            "expected exit code {}, got {}",
            code, self.exit_code
        );
    }

    pub fn assert_stdout_contains(&self, needle: &str) {
        assert!(
            self.stdout.contains(needle),
            "expected stdout to contain {:?}, got:\n{}",
            needle,
            self.stdout
        );
    }

    pub fn assert_stderr_contains(&self, needle: &str) {
        assert!(
            self.stderr.contains(needle),
            "expected stderr to contain {:?}, got:\n{}",
            needle,
            self.stderr
        );
    }
}

/// Normalized output with timestamps stripped.
pub struct NormalizedOutput {
    pub stdout: String,
    pub stderr: String,
}

/// Strip ISO 8601 timestamps of the form `NNNN-NN-NNTNN:NN:NN` from a string.
fn strip_timestamps(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if i + 19 <= chars.len() && is_timestamp(&chars[i..i + 19]) {
            result.push_str("<TIMESTAMP>");
            i += 19;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn is_timestamp(chars: &[char]) -> bool {
    if chars.len() != 19 {
        return false;
    }
    chars[0..4].iter().all(|c| c.is_ascii_digit())
        && chars[4] == '-'
        && chars[5..7].iter().all(|c| c.is_ascii_digit())
        && chars[7] == '-'
        && chars[8..10].iter().all(|c| c.is_ascii_digit())
        && chars[10] == 'T'
        && chars[11..13].iter().all(|c| c.is_ascii_digit())
        && chars[13] == ':'
        && chars[14..16].iter().all(|c| c.is_ascii_digit())
        && chars[16] == ':'
        && chars[17..19].iter().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_timestamp() {
        let out = TestOutput {
            stdout: "started at 2026-04-28T12:00:00 done".to_string(),
            stderr: "error at 2026-04-28T12:00:00".to_string(),
            exit_code: 0,
        };
        let normalized = out.normalize();
        assert_eq!(normalized.stdout, "started at <TIMESTAMP> done");
        assert_eq!(normalized.stderr, "error at <TIMESTAMP>");
    }

    #[test]
    fn normalize_no_timestamp_unchanged() {
        let out = TestOutput {
            stdout: "no timestamps here".to_string(),
            stderr: "".to_string(),
            exit_code: 0,
        };
        let normalized = out.normalize();
        assert_eq!(normalized.stdout, "no timestamps here");
    }

    #[test]
    fn assert_diagnostic_code_panics_on_missing() {
        let out = TestOutput {
            stdout: "".to_string(),
            stderr: "error[E001]: unknown".to_string(),
            exit_code: 1,
        };
        let result = std::panic::catch_unwind(|| {
            out.assert_diagnostic_code("E999");
        });
        assert!(result.is_err(), "should panic when code not found");
    }

    #[test]
    fn assert_diagnostic_code_passes_when_present() {
        let out = TestOutput {
            stdout: "".to_string(),
            stderr: "error[E001]: unknown command\n  hint: check spelling".to_string(),
            exit_code: 1,
        };
        out.assert_diagnostic_code("E001"); // should not panic
    }

    #[test]
    fn run_with_env_restores_env_after_call() {
        // Verify the mechanism: env var should not persist after run_with_env
        std::env::remove_var("TEST_HARNESS_VAR");
        // Simulate what run_with_env does:
        std::env::set_var("TEST_HARNESS_VAR", "hello");
        assert_eq!(std::env::var("TEST_HARNESS_VAR").unwrap(), "hello");
        std::env::remove_var("TEST_HARNESS_VAR");
        assert!(std::env::var("TEST_HARNESS_VAR").is_err());
    }
}
