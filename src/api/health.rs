use crate::parser::error_codes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::atomic::Ordering;

#[derive(Clone)]
pub struct HealthState {
    pub shutdown: tokio_util::sync::CancellationToken,
    pub ready_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub readiness_check: super::ReadinessCheck,
    pub crate_version: &'static str,
}

pub async fn healthz(State(state): State<HealthState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(json!({"status":"ok","version": state.crate_version})),
    )
}

pub async fn readyz(State(state): State<HealthState>) -> Response {
    // Fast path: during shutdown, return 503 immediately (no awaits).
    if !state.ready_flag.load(Ordering::SeqCst) || state.shutdown.is_cancelled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"status":"not_ready","checks":{"shutdown":true},"error_code": error_codes::E_API_NOT_READY})),
        )
            .into_response();
    }

    let report = (state.readiness_check)().await;
    if report.ready {
        (StatusCode::OK, axum::Json(json!({"status":"ready"}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"status":"not_ready","checks": report.checks,"error_code": error_codes::E_API_NOT_READY})),
        )
            .into_response()
    }
}
