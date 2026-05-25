use crate::parser::error_codes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Clone)]
pub struct HealthState {
    pub shutdown: tokio_util::sync::CancellationToken,
    pub shutdown_readiness: Arc<std::sync::atomic::AtomicBool>,
    pub readiness_check: super::ReadinessCheck,
    pub crate_version: String,
}

pub async fn healthz(State(state): State<HealthState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(json!({"status":"ok","version": state.crate_version.clone()})),
    )
}

pub async fn readyz(State(state): State<HealthState>) -> Response {
    // Fast path: during shutdown, return 503 immediately (no awaits).
    if state.shutdown_readiness.load(Ordering::Relaxed) || state.shutdown.is_cancelled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(
                json!({"status":"not_ready","checks":{"error_code": error_codes::E_API_NOT_READY}}),
            ),
        )
            .into_response();
    }

    let report = (state.readiness_check)().await;
    if report.ready {
        (StatusCode::OK, axum::Json(json!({"status":"ready"}))).into_response()
    } else {
        let mut checks = report.checks;
        // Keep the response envelope fixed: status + checks only, but always include E021.
        checks.insert(
            "error_code".to_string(),
            serde_json::Value::String(error_codes::E_API_NOT_READY.to_string()),
        );
        (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"status":"not_ready","checks": checks})),
        )
            .into_response()
    }
}
