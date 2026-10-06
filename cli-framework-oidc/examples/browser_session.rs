//! Local development host; register http://127.0.0.1:4311/callback with the issuer.
//! Set OIDC_EXAMPLE_ISSUER, OIDC_EXAMPLE_CLIENT and OIDC_EXAMPLE_AUDIENCE.
//! The ephemeral key intentionally invalidates cookies on restart.
use cli_framework::axum::{routing, Extension, Json, Router};
use cli_framework_oidc::browser::{
    AudiencePolicy, OidcBrowserSession, OidcBrowserSessionConfig, OidcClaims, SessionKey,
};
use rand::RngCore;
use tower::Layer;

fn application(
    config: OidcBrowserSessionConfig,
) -> Result<Router, cli_framework_oidc::OidcConfigError> {
    let audience = config.audience.clone();
    let session = OidcBrowserSession::new(config)?;
    let ui = session.browser_layer();
    let api = Router::new().route("/identity", routing::get(identity));
    Ok(ui
        .callback_router
        .nest_service("/api", session.api_layer(audience).layer(api))
        .fallback_service(ui.layer.layer(Router::new().route(
            "/",
            routing::get(|| async { "Authenticated browser session" }),
        ))))
}

async fn identity(Extension(claims): Extension<OidcClaims>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"subject": claims.sub, "issuer": claims.iss}))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let issuer = std::env::var("OIDC_EXAMPLE_ISSUER")?;
    let client = std::env::var("OIDC_EXAMPLE_CLIENT")?;
    let audience = std::env::var("OIDC_EXAMPLE_AUDIENCE")?;
    let mut key = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    let config = OidcBrowserSessionConfig::new(
        issuer,
        client,
        "http://127.0.0.1:4311/callback",
        SessionKey::from_bytes(key),
        AudiencePolicy::Require(audience),
    );
    let app = application(config)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4311").await?;
    cli_framework::axum::serve(listener, app).await?;
    Ok(())
}
