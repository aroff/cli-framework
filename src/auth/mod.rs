//! Optional authentication module
//!
//! Provides built-in authentication mechanisms that applications can opt-in to use.
//! Applications can also implement their own authentication via AppContext.

pub mod login;
pub mod rbac;
pub mod token;

pub use login::LoginScreen;
pub use rbac::RbacManager;
pub use token::TokenManager;
