//! API Server Example
//!
//! Runs a versioned HTTP API with health endpoints and embedded Swagger UI:
//!
//! - `GET /healthz`
//! - `GET /readyz`
//! - `GET /api/v1/hello`
//! - `GET /api/v2/hello`
//! - `GET /api/v1/openapi.json`  (requires `api-swagger`)
//! - `GET /api/v2/openapi.json`  (requires `api-swagger`)
//! - `GET /api/docs`             (Swagger UI, requires `api-swagger`)
//!
//! ```bash
//! cargo run --example with_api --features "api-server,api-swagger"
//! curl -sS http://127.0.0.1:8082/healthz
//! curl -sS http://127.0.0.1:8082/api/v1/hello
//! curl -sS http://127.0.0.1:8082/api/v1/openapi.json
//! curl -sS http://127.0.0.1:8082/api/docs
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
            #[cfg(feature = "api-swagger")]
            openapi: Some(serde_json::json!({
                "openapi": "3.0.3",
                "info": { "title": "Example API", "version": "1.0.0" },
                "paths": {
                    "/hello": {
                        "get": {
                            "summary": "Hello from v1",
                            "responses": { "200": { "description": "OK" } }
                        }
                    }
                }
            })),
        })
        .version(ApiVersion {
            name: ApiVersionName::parse("v2")?,
            router: v2,
            stability: Stability::Stable,
            deprecation: None,
            #[cfg(feature = "api-swagger")]
            openapi: Some(serde_json::json!({
                "openapi": "3.0.3",
                "info": { "title": "Example API", "version": "2.0.0" },
                "paths": {
                    "/hello": {
                        "get": {
                            "summary": "Hello from v2",
                            "responses": { "200": { "description": "OK" } }
                        }
                    }
                }
            })),
        })
        .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v2")?))
        .build();

    server.serve("127.0.0.1:8082").await
}
