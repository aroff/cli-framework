//! App-layer diagnostic reporter. Lives in the app layer so the parser module remains
//! side-effect-free.

use crate::parser::diagnostic::Diagnostic;
use std::io::Write;

#[cfg(feature = "testkit")]
use std::cell::RefCell;

/// Writes structured diagnostics to stderr (or to an in-process capture buffer
/// when the `testkit` feature is active).
pub struct DiagnosticReporter;

// ── optional in-process stderr capture (testkit only) ─────────────────────────
//
// The buffer is **thread-local**, not a process-global. `cargo test` runs test
// functions concurrently across threads, and each `CliTestHarness::run()` wraps
// its dispatch in `begin_capture()` / `take_capture()`. A shared global buffer
// would let parallel tests clobber each other's captures (one test resetting the
// buffer mid-flight, another's diagnostic landing in the wrong buffer), producing
// flaky empty-stderr reads. Default `#[tokio::test]` runs on a current-thread
// runtime, so all of a single test's work — including diagnostic reporting during
// dispatch — stays on the same thread, making thread-local isolation correct.

#[cfg(feature = "testkit")]
thread_local! {
    static STDERR_CAPTURE: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

impl DiagnosticReporter {
    /// Write a single diagnostic to stderr in the format:
    /// `"error[{code}]: {message}\n  hint: {suggestion}\n"`
    pub fn report(diagnostic: &Diagnostic) {
        let mut msg = format!("error[{}]: {}\n", diagnostic.code, diagnostic.message);
        if let Some(ref hint) = diagnostic.suggestion {
            msg.push_str(&format!("  hint: {}\n", hint));
        }

        #[cfg(feature = "testkit")]
        {
            let captured = STDERR_CAPTURE.with(|cell| {
                if let Some(ref mut v) = *cell.borrow_mut() {
                    v.extend_from_slice(msg.as_bytes());
                    true
                } else {
                    false
                }
            });
            if captured {
                return;
            }
        }

        let mut stderr = std::io::stderr();
        let _ = stderr.write_all(msg.as_bytes());
    }

    /// Write all diagnostics to stderr.
    pub fn report_all(diagnostics: &[Diagnostic]) {
        for d in diagnostics {
            Self::report(d);
        }
    }

    /// Begin capturing stderr output into an internal buffer.
    /// Only available with the `testkit` feature.
    #[cfg(feature = "testkit")]
    pub(crate) fn begin_capture() {
        STDERR_CAPTURE.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
    }

    /// Consume and return the captured stderr output.
    /// Clears the capture buffer.
    #[cfg(feature = "testkit")]
    pub(crate) fn take_capture() -> String {
        let data = STDERR_CAPTURE.with(|cell| cell.borrow_mut().take().unwrap_or_default());
        String::from_utf8(data).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::diagnostic::DiagnosticCategory;

    #[test]
    fn report_formats_code_in_output() {
        // We can't easily capture stderr in a unit test without testkit,
        // so just exercise the API path to ensure it does not panic.
        let d = Diagnostic {
            code: "E001",
            category: DiagnosticCategory::Parse,
            message: "test message".to_string(),
            suggestion: Some("try this".to_string()),
            span: None,
        };
        // Should not panic:
        DiagnosticReporter::report(&d);
    }

    #[cfg(feature = "testkit")]
    #[test]
    fn report_captures_when_testkit_enabled() {
        DiagnosticReporter::begin_capture();
        let d = Diagnostic {
            code: "E007",
            category: DiagnosticCategory::Parse,
            message: "collision".to_string(),
            suggestion: None,
            span: None,
        };
        DiagnosticReporter::report(&d);
        let captured = DiagnosticReporter::take_capture();
        assert!(
            captured.contains("E007"),
            "captured stderr should contain code E007, got: {:?}",
            captured
        );
    }
}
