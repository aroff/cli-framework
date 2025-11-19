//! AppContext trait and helpers
//!
//! AppContext represents application-owned state and service clients.

/// Application context trait
///
/// Applications implement this trait to provide their own state and service clients.
/// The framework uses this to pass context to views, datasources, and commands.
pub trait AppContext {
    // Applications define their own structure
    // Framework only requires the trait to exist
}

