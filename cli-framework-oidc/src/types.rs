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
    pub iss: String,
    pub aud: Vec<String>,
    pub exp: i64,
    pub iat: Option<i64>,
    pub nbf: Option<i64>,
    pub preferred_username: Option<String>,
    pub email: Option<String>,
    pub scopes: Vec<String>,
    pub roles: Vec<String>,
    pub raw: JsonValue,
}
