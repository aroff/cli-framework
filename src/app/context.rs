//! AppContext trait and helpers
//!
//! AppContext represents application-owned state and service clients.

/// Application context trait
///
/// Applications implement this trait to provide their own state and service clients.
/// The framework uses this to pass context to views, datasources, and commands.
///
/// # Thread Safety
///
/// AppContext implementations must be `Send + Sync` to ensure thread safety in the async runtime.
/// This allows the framework to safely share context across async tasks and threads.
pub trait AppContext: Send + Sync {
    // Applications define their own structure
    // Framework only requires the trait to exist
}
