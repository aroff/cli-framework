//! API Server Example
//!
//! Runs a versioned HTTP API with fixed health endpoints:
//!
//! - `GET /healthz`
//! - `GET /readyz`
//! - `GET /api/v1/hello`
//! - `GET /api/v2/hello`
//!
//! ```bash
//! cargo run --example with_api --features "api-server"
//! curl -sS http://127.0.0.1:8082/healthz
//! curl -sS http://127.0.0.1:8082/api/v1/hello
//! curl -sS http://127.0.0.1:8082/api/v2/hello
//! ```

use cli_framework::api::{ApiServerBuilder, ApiVersion, ApiVersionName, DefaultVersion, Stability};
use cli_framework::axum::{routing::get, Router};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let v1 = Router::new().route("/hello", get(|| async { "hello from v1" }));
    let v2 = Router::new().route("/hello", get(|| async { "hello from v2" }));

    let server = ApiServerBuilder::new()
        .version(ApiVersion {
            name: ApiVersionName::parse("v1")?,
            router: v1,
            stability: Stability::Stable,
            deprecation: None,
        })
        .version(ApiVersion {
            name: ApiVersionName::parse("v2")?,
            router: v2,
            stability: Stability::Stable,
            deprecation: None,
        })
        .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v2")?))
        .build();

    server.serve("127.0.0.1:8082").await
}
