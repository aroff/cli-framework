/// Application-level metadata shown in the help header.
///
/// All fields use `&'static str` consistent with the `Command` struct convention.
#[derive(Debug, Clone, Copy)]
pub struct AppMeta {
    /// Short program name (e.g. "mycli"). Shown in header line.
    pub name: &'static str,
    /// Semantic version string (e.g. "1.0.0"). Shown in Options block.
    pub version: &'static str,
    /// One-line description. Shown alongside `name` in header.
    pub description: &'static str,
    /// Optional custom usage line.
    /// Defaults to `"<name> [OPTIONS] <command>"` when `None`.
    pub usage: Option<&'static str>,
}
