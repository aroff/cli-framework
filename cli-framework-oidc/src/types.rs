/// Shared OIDC types used by both `server` and `browser` features.
use serde_json::Value as JsonValue;

/// Audience validation policy for JWT tokens.
#[derive(Clone, Debug)]
pub enum AudiencePolicy {
    /// Token is valid only if its `aud` contains this exact value.
    Require(String),
    /// Token is valid if its `aud` contains **any** of these values.
    RequireAny(Vec<String>),
    Unchecked,
}

/// Extracted and validated OIDC claims, inserted into request extensions.
#[derive(Clone, Debug)]
pub struct OidcClaims {
    pub sub: String,
    /// The issuer that validated the token, as configured (normalized). With
    /// several trusted issuers this is how a handler tells them apart.
    pub iss: String,
    pub aud: Vec<String>,
    pub exp: i64,
    pub iat: Option<i64>,
    pub nbf: Option<i64>,
    pub preferred_username: Option<String>,
    pub email: Option<String>,
    pub scopes: Vec<String>,
    /// Strings at the issuer's roles claim path (default `realm_access.roles`).
    pub roles: Vec<String>,
    /// Strings at the issuer's groups claim path; empty when none is configured.
    pub groups: Vec<String>,
    pub raw: JsonValue,
}
