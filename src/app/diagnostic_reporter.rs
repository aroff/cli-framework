//! App-layer diagnostic reporter. Lives in the app layer so the parser module remains
//! side-effect-free.

use crate::parser::diagnostic::Diagnostic;

#[cfg(feature = "testkit")]
use std::sync::{Mutex, OnceLock};

/// Writes structured diagnostics to stderr (or to an in-process capture buffer
/// when the `testkit` feature is active).
pub struct DiagnosticReporter;

// ── optional in-process stderr capture (testkit only) ─────────────────────────

#[cfg(feature = "testkit")]
static STDERR_CAPTURE: OnceLock<Mutex<Option<Vec<u8>>>> = OnceLock::new();

#[cfg(feature = "testkit")]
fn stderr_capture_buf() -> &'static Mutex<Option<Vec<u8>>> {
    STDERR_CAPTURE.get_or_init(|| Mutex::new(None))
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
            let mut buf = stderr_capture_buf().lock().unwrap();
            if let Some(ref mut v) = *buf {
                v.extend_from_slice(msg.as_bytes());
                return;
            }
        }

        eprint!("{}", msg);
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
        let mut buf = stderr_capture_buf().lock().unwrap();
        *buf = Some(Vec::new());
    }

    /// Consume and return the captured stderr output.
    /// Clears the capture buffer.
    #[cfg(feature = "testkit")]
    pub(crate) fn take_capture() -> String {
        let mut buf = stderr_capture_buf().lock().unwrap();
        let data = buf.take().unwrap_or_default();
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
