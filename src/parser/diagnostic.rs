/// A structured diagnostic produced by parsing or validation.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Stable error code, e.g. `"E001"`.
    pub code: &'static str,
    pub category: DiagnosticCategory,
    pub message: String,
    /// Actionable hint for the user.
    pub suggestion: Option<String>,
    /// The raw argv token that triggered the error, if applicable.
    pub span: Option<String>,
}

/// Which phase produced this diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub enum DiagnosticCategory {
    Parse,
    Spec,
    Validation,
    Completion,
}
