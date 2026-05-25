use crate::parser::error_codes;
use axum::extract::Path;
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::{ApiVersionName, DefaultVersion};

pub fn redirect_location(default: &ApiVersionName, rest: &str, uri: &Uri) -> String {
    // `rest` is captured from the request path after `/api`, and may be empty.
    let mut path = format!("/api/{}", default.as_str());
    if !rest.is_empty() {
        if !rest.starts_with('/') {
            path.push('/');
        }
        path.push_str(rest);
    }
    if let Some(q) = uri.query() {
        path.push('?');
        path.push_str(q);
    }
    path
}

pub async fn handle_unversioned(
    default: DefaultVersion,
    available_versions: Vec<String>,
    uri: Uri,
    Path(rest): Path<String>,
) -> Response {
    match default {
        DefaultVersion::Pinned(v) => {
            let loc = redirect_location(&v, &rest, &uri);
            let mut resp = StatusCode::PERMANENT_REDIRECT.into_response();
            resp.headers_mut().insert(
                axum::http::header::LOCATION,
                HeaderValue::from_str(&loc).unwrap(),
            );
            resp
        }
        DefaultVersion::None => (
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "error_code": error_codes::E_API_VERSION_REQUIRED,
                "message": "missing api version segment; use /api/{version}/...",
                "available_versions": available_versions,
            })),
        )
            .into_response(),
    }
}
