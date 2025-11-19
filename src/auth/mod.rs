//! Optional authentication module
//!
//! Provides built-in authentication mechanisms that applications can opt-in to use.
//! Applications can also implement their own authentication via AppContext.

pub mod login;
pub mod token;
pub mod rbac;

pub use login::LoginScreen;
pub use token::TokenManager;
pub use rbac::RbacManager;

