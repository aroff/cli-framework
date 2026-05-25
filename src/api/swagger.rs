//! Swagger UI serving and per-version OpenAPI spec endpoint.
//! Feature-gated: `api-swagger`.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde_json::Value;
use utoipa_swagger_ui::{Config, SwaggerUi, Url};

/// State shared by per-version OpenAPI handlers.
#[derive(Clone)]
pub(crate) struct OpenApiState {
    /// Pre-serialized, servers-patched OpenAPI document.
    pub doc_json: String,
}

/// Handler: returns the pre-serialized, servers-patched OpenAPI document.
pub(crate) async fn openapi_json_handler(State(state): State<OpenApiState>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        state.doc_json,
    )
        .into_response()
}

/// Patch `servers:` in `doc` to `[{"url": "/api/{version}"}]` and serialize to JSON.
pub(crate) fn patch_and_serialize(
    mut doc: Value,
    version_name: &str,
) -> Result<String, serde_json::Error> {
    let prefix = format!("/api/{}", version_name);
    doc["servers"] = serde_json::json!([{"url": prefix}]);
    serde_json::to_string(&doc)
}

/// Build the swagger router: per-version spec routes + embedded Swagger UI at /api/docs.
///
/// `swagger_versions`: `(version_name, pre-serialized patched doc JSON)`
/// `primary_version`: version name to open by default in the UI
pub(crate) fn build_swagger_router(
    swagger_versions: Vec<(String, String)>,
    primary_version: &str,
) -> Router {
    let mut router = Router::new();

    // Register our own handler for each version's openapi.json.
    for (name, doc_json) in &swagger_versions {
        let state = OpenApiState {
            doc_json: doc_json.clone(),
        };
        let path = format!("/api/{}/openapi.json", name);
        router = router.route(&path, get(openapi_json_handler).with_state(state));
    }

    // Build Swagger UI at /api/docs with a custom Config pointing to our spec routes.
    // We use Config::new (not external_url_unchecked) so the SwaggerUI does not register
    // its own conflicting routes — our openapi_json_handler routes above serve the specs.
    let config_urls: Vec<Url<'static>> = swagger_versions
        .iter()
        .map(|(name, _)| {
            let spec_url = format!("/api/{}/openapi.json", name);
            let is_primary = name == primary_version;
            // Box::leak: acceptable for build-time strings that live for the program's lifetime.
            let name_static: &'static str = Box::leak(name.clone().into_boxed_str());
            let url_static: &'static str = Box::leak(spec_url.into_boxed_str());
            if is_primary {
                Url::with_primary(name_static, url_static, true)
            } else {
                Url::new(name_static, url_static)
            }
        })
        .collect();

    let ui = SwaggerUi::new("/api/docs").config(Config::new(config_urls));
    router.merge(Router::from(ui))
}
