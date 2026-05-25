use axum::middleware::Next;
use axum::response::Response;
use axum::Router;
use chrono::{DateTime, Utc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct HeaderConfig {
    pub api_version: String,
    pub sunset: Option<DateTime<Utc>>,
    pub docs_url: Option<String>,
}

pub fn apply_versioned_headers(router: Router, cfg: HeaderConfig) -> Router {
    router.layer(axum::middleware::from_fn(
        move |req: axum::http::Request<axum::body::Body>, next: Next| {
            let cfg = cfg.clone();
            async move {
                let mut resp: Response = next.run(req).await;
                let headers = resp.headers_mut();

                headers.insert(
                    axum::http::HeaderName::from_bytes(b"X-API-Version")
                        .expect("X-API-Version header name must be valid"),
                    axum::http::HeaderValue::from_str(&cfg.api_version)
                        .unwrap_or(axum::http::HeaderValue::from_static("")),
                );

                if let Some(sunset) = cfg.sunset {
                    headers.insert(
                        axum::http::HeaderName::from_bytes(b"Deprecation")
                            .expect("Deprecation header name must be valid"),
                        axum::http::HeaderValue::from_static("true"),
                    );

                    let secs = sunset.timestamp();
                    if secs >= 0 {
                        let nanos = sunset.timestamp_subsec_nanos() as u64;
                        let st: SystemTime = UNIX_EPOCH
                            + Duration::from_secs(secs as u64)
                            + Duration::from_nanos(nanos);
                        let val = httpdate::fmt_http_date(st);
                        if let Ok(hv) = axum::http::HeaderValue::from_str(&val) {
                            headers.insert(
                                axum::http::HeaderName::from_bytes(b"Sunset")
                                    .expect("Sunset header name must be valid"),
                                hv,
                            );
                        }
                    }

                    if let Some(url) = cfg.docs_url.as_deref() {
                        let link_val = format!("<{}>; rel=\"deprecation\"", url);
                        if let Ok(hv) = axum::http::HeaderValue::from_str(&link_val) {
                            headers.insert(axum::http::header::LINK, hv);
                        }
                    }
                }

                resp
            }
        },
    ))
}
