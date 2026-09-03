//! PKCE (RFC 7636) code verifier and challenge generation.
//!
//! Re-export of [`crate::pkce`], kept as a module path because
//! `browser::pkce::{generate_verifier, derive_challenge}` is public API that
//! downstream code and `tests/browser_validation.rs` import directly. The
//! implementation is shared with the `client` feature so both flows are backed
//! by the same audited `OsRng` code.

pub use crate::pkce::{derive_challenge, generate_state, generate_verifier};
