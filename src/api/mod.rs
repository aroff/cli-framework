//! Built-in HTTP API hosting with versioned routing (`/api/{version}/...`).
//!
//! This module is behind the `api-server` feature flag.

mod box_clone_layer;
mod headers;
mod health;
mod versioning;

use crate::parser::error_codes;
use axum::http::Uri;
use axum::routing::{any, get};
use axum::Router;
use chrono::{DateTime, Utc};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tower::Layer;

pub use box_clone_layer::BoxCloneLayer;
pub use headers::{apply_versioned_headers, HeaderConfig};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApiVersionName(String);

impl ApiVersionName {
    pub fn parse(name: impl Into<String>) -> Result<Self, ApiServerError> {
        let name = name.into();
        if name == "docs" || name == "openapi.json" {
            return Err(ApiServerError::new(
                error_codes::E_API_VERSION_RESERVED,
                format!("version name '{name}' is reserved under /api"),
            ));
        }
        // Required shape: ^v\d+(?:beta\d+|alpha\d+)?$
        // Examples: v1, v2, v2beta1, v3alpha1
        static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"^v\d+(?:beta\d+|alpha\d+)?$").unwrap());
        if re.is_match(&name) {
            Ok(Self(name))
        } else {
            Err(ApiServerError::new(
                error_codes::E_API_VERSION_INVALID,
                format!("invalid api version name: '{name}'"),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Internal escape hatch for tests and advanced callers.
    ///
    /// Prefer `parse()` unless you are intentionally deferring validation to `ApiServerBuilder::build()`.
    pub fn new_unchecked(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stability {
    Stable,
    Beta,
    Alpha,
}

#[derive(Debug, Clone)]
pub struct DeprecationInfo {
    pub sunset: DateTime<Utc>,
    pub docs_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiVersion {
    pub name: ApiVersionName,
    pub router: axum::Router,
    pub stability: Stability,
    pub deprecation: Option<DeprecationInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultVersion {
    None,
    Pinned(ApiVersionName),
}

#[derive(Debug, Clone)]
pub struct ReadinessReport {
    pub ready: bool,
    pub checks: BTreeMap<String, serde_json::Value>,
}

pub type ReadinessCheckFuture = Pin<Box<dyn Future<Output = ReadinessReport> + Send + 'static>>;
pub type ReadinessCheck = Arc<dyn Fn() -> ReadinessCheckFuture + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct ApiMount {
    pub path: String,
    pub router: axum::Router,
}

#[derive(Debug, Clone)]
pub struct ApiServerError {
    code: &'static str,
    message: String,
}

impl ApiServerError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for ApiServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiServerError {}

fn panic_config(code: &'static str, msg: impl AsRef<str>) -> ! {
    panic!("{}: {}", code, msg.as_ref())
}

pub struct ApiServerBuilder {
    versions: BTreeMap<ApiVersionName, ApiVersion>,
    mounts: Vec<ApiMount>,
    default_version: DefaultVersion,
    cors: Option<tower_http::cors::CorsLayer>,
    // Type-erased layer: MUST be clonable and applicable to the router.
    auth: Option<BoxCloneLayer<axum::Router>>,
    readiness_check: ReadinessCheck,
    protect_health: bool,
    reserved_prefixes: BTreeSet<String>,
    mcp_router: Option<axum::Router>,
}

impl std::fmt::Debug for ApiServerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiServerBuilder")
            .field(
                "versions",
                &self.versions.keys().map(|v| v.as_str()).collect::<Vec<_>>(),
            )
            .field(
                "mounts",
                &self
                    .mounts
                    .iter()
                    .map(|m| m.path.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("default_version", &self.default_version)
            .field("cors", &self.cors.is_some())
            .field("auth", &self.auth.is_some())
            .field("protect_health", &self.protect_health)
            .field("reserved_prefixes", &self.reserved_prefixes)
            .finish()
    }
}

impl Default for ApiServerBuilder {
    fn default() -> Self {
        Self {
            versions: BTreeMap::new(),
            mounts: Vec::new(),
            default_version: DefaultVersion::None,
            cors: None,
            auth: None,
            readiness_check: Arc::new(|| {
                Box::pin(async {
                    ReadinessReport {
                        ready: true,
                        checks: BTreeMap::new(),
                    }
                })
            }),
            protect_health: false,
            reserved_prefixes: BTreeSet::new(),
            mcp_router: None,
        }
    }
}

impl ApiServerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn version(mut self, v: ApiVersion) -> Self {
        if self.versions.contains_key(&v.name) {
            panic_config(
                error_codes::E_API_DUP_VERSION,
                format!("duplicate api version '{}'", v.name.as_str()),
            );
        }
        self.versions.insert(v.name.clone(), v);
        self
    }

    pub fn mount(mut self, path: &str, router: axum::Router) -> Self {
        self.mounts.push(ApiMount {
            path: path.to_string(),
            router,
        });
        self
    }

    pub fn default_version(mut self, d: DefaultVersion) -> Self {
        self.default_version = d;
        self
    }

    pub fn cors(mut self, layer: tower_http::cors::CorsLayer) -> Self {
        self.cors = Some(layer);
        self
    }

    pub fn auth(mut self, layer: BoxCloneLayer<axum::Router>) -> Self {
        self.auth = Some(layer);
        self
    }

    /// Enable MCP coexistence at the fixed `/mcp` path.
    ///
    /// Typical usage:
    /// - `cli_framework::mcp::transport_http::mcp_axum_router(tool_registry, "/mcp")`
    /// - `cli_framework::mcp::build_mcp_axum_router(..., "/mcp")`
    pub fn mcp_router(mut self, router: axum::Router) -> Self {
        self.mcp_router = Some(router);
        self
    }

    pub fn readiness_check(mut self, check: ReadinessCheck) -> Self {
        self.readiness_check = check;
        self
    }

    pub fn protect_health(mut self, yes: bool) -> Self {
        self.protect_health = yes;
        self
    }

    pub fn reserved_prefixes(mut self, prefixes: &[&str]) -> Self {
        for p in prefixes {
            self.reserved_prefixes.insert(p.to_string());
        }
        self
    }

    pub fn build(self) -> ApiServer {
        // Validate versions: non-empty, unique by ApiVersionName, and name shape.
        if self.versions.is_empty() {
            panic_config(error_codes::E_API_NO_VERSIONS, "no api versions registered");
        }

        let mut validated_versions: BTreeMap<ApiVersionName, ApiVersion> = BTreeMap::new();
        for (name, v) in self.versions.into_iter() {
            // Validate name regex and reserved segments under /api.
            let parsed = ApiVersionName::parse(name.0.clone())
                .unwrap_or_else(|e| panic_config(e.code(), e.to_string()));

            if validated_versions.contains_key(&parsed) {
                panic_config(
                    error_codes::E_API_DUP_VERSION,
                    format!("duplicate api version '{}'", parsed.as_str()),
                );
            }
            validated_versions.insert(parsed.clone(), ApiVersion { name: parsed, ..v });
        }

        if let DefaultVersion::Pinned(v) = &self.default_version {
            if !validated_versions.contains_key(v) {
                panic_config(
                    error_codes::E_API_DEFAULT_UNKNOWN,
                    format!("default version '{}' is not registered", v.as_str()),
                );
            }
        }

        // Normalize mount paths, then check collisions.
        let mut mounts: Vec<ApiMount> = Vec::with_capacity(self.mounts.len());
        for m in self.mounts.into_iter() {
            let p = normalize_mount_path(&m.path)
                .unwrap_or_else(|e| panic_config(error_codes::E_API_MOUNT_COLLISION, e));
            mounts.push(ApiMount {
                path: p,
                router: m.router,
            });
        }

        // The primary API must only be served via versioned routes, so never allow mounting at `/` or `/api`.
        for m in mounts.iter() {
            if m.path == "/" {
                panic_config(
                    error_codes::E_API_MOUNT_COLLISION,
                    "mount('/') is not allowed; use /api/{version}/... for primary APIs",
                );
            }
            if m.path == "/api" {
                panic_config(
                    error_codes::E_API_MOUNT_COLLISION,
                    "mount('/api', ...) is not allowed; use version registration instead",
                );
            }
        }

        let version_prefixes: Vec<String> = validated_versions
            .keys()
            .map(|v| format!("/api/{}", v.as_str()))
            .collect();

        let mut reserved_api_prefixes = vec!["/api/docs".to_string()];
        for v in validated_versions.keys() {
            reserved_api_prefixes.push(format!("/api/{}/openapi.json", v.as_str()));
        }

        // Fixed host paths.
        let fixed_paths = [
            "/api".to_string(),
            "/healthz".to_string(),
            "/readyz".to_string(),
            "/mcp".to_string(),
        ];
        let fixed_prefixes = ["/api".to_string(), "/mcp".to_string()];

        // Check mount collisions with fixed paths and reserved prefixes.
        for m in mounts.iter() {
            for fixed in fixed_paths.iter().chain(reserved_api_prefixes.iter()) {
                if paths_collide(&m.path, fixed) {
                    panic_config(
                        error_codes::E_API_MOUNT_COLLISION,
                        format!(
                            "mount path '{}' collides with reserved path '{}'",
                            m.path, fixed
                        ),
                    );
                }
            }
            for vp in version_prefixes.iter() {
                if paths_collide(&m.path, vp) {
                    panic_config(
                        error_codes::E_API_MOUNT_COLLISION,
                        format!(
                            "mount path '{}' collides with api version prefix '{}'",
                            m.path, vp
                        ),
                    );
                }
            }
            for fp in fixed_prefixes.iter() {
                if paths_collide(&m.path, fp) {
                    panic_config(
                        error_codes::E_API_MOUNT_COLLISION,
                        format!(
                            "mount path '{}' collides with reserved host prefix '{}'",
                            m.path, fp
                        ),
                    );
                }
            }
        }

        // Check reserved prefixes (caller-supplied) for collisions.
        for rp in self.reserved_prefixes.iter() {
            let rp = normalize_mount_path(rp)
                .unwrap_or_else(|e| panic_config(error_codes::E_API_MOUNT_COLLISION, e));
            for fixed in fixed_paths.iter() {
                if paths_collide(&rp, fixed) {
                    panic_config(
                        error_codes::E_API_MOUNT_COLLISION,
                        format!(
                            "reserved prefix '{}' collides with reserved host path '{}'",
                            rp, fixed
                        ),
                    );
                }
            }
            for fp in fixed_prefixes.iter() {
                if paths_collide(&rp, fp) {
                    panic_config(
                        error_codes::E_API_MOUNT_COLLISION,
                        format!(
                            "reserved prefix '{}' collides with reserved host prefix '{}'",
                            rp, fp
                        ),
                    );
                }
            }
        }

        let shutdown = CancellationToken::new();

        let available_versions: Vec<String> = validated_versions
            .keys()
            .map(|v| v.as_str().to_string())
            .collect();

        let health_state = health::HealthState {
            shutdown: shutdown.clone(),
            readiness_check: Arc::clone(&self.readiness_check),
            crate_version: env!("CARGO_PKG_VERSION"),
        };

        let mut router = Router::new();

        // Health and readiness are always present at root.
        let mut health_router = Router::new()
            .route("/healthz", get(health::healthz))
            .route("/readyz", get(health::readyz))
            .with_state(health_state);

        // API root: /api/{version}/...
        let mut api_root = Router::new();
        for (_, v) in validated_versions.iter() {
            let mut vr = v.router.clone();

            // Attach version identity + deprecation headers.
            let hc = HeaderConfig {
                api_version: v.name.as_str().to_string(),
                sunset: v.deprecation.as_ref().map(|d| d.sunset),
                docs_url: v.deprecation.as_ref().and_then(|d| d.docs_url.clone()),
            };
            vr = headers::apply_versioned_headers(vr, hc);

            // Minimal host-provided request/response tracing (streaming-safe).
            vr = vr.layer(axum::middleware::from_fn(
                |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| async move {
                let start = std::time::Instant::now();
                let method = req.method().clone();
                let path = req.uri().path().to_string();
                let resp: axum::response::Response = next.run(req).await;
                log::info!(
                    "api {} {} -> {} ({:?})",
                    method,
                    path,
                    resp.status(),
                    start.elapsed()
                );
                resp
            },
            ));

            // Apply shared layers to the version router.
            if let Some(cors) = self.cors.clone() {
                vr = vr.layer(cors);
            }
            if let Some(auth) = self.auth.clone() {
                vr = auth.layer(vr);
            }

            api_root = api_root.nest(&format!("/{}", v.name.as_str()), vr);
        }

        // Requests to `/api/...` without a version segment are handled by a host endpoint.
        // Since `/api` is a nested router, we implement this behavior in the `/api` router itself.
        let default_version = self.default_version.clone();
        let av = available_versions.clone();
        api_root = api_root.route(
            "/",
            any(move |uri: Uri| async move {
                versioning::handle_unversioned(
                    default_version.clone(),
                    av.clone(),
                    uri,
                    axum::extract::Path("".to_string()),
                )
                .await
            }),
        );
        let default_version = self.default_version.clone();
        let av = available_versions.clone();
        api_root = api_root.fallback(any(move |uri: Uri| async move {
            let rest = uri.path().trim_start_matches('/').to_string();
            versioning::handle_unversioned(
                default_version.clone(),
                av.clone(),
                uri,
                axum::extract::Path(rest),
            )
            .await
        }));

        router = router.nest("/api", api_root);

        // `/api/` doesn't match the nested router at `/api`, so handle it explicitly.
        let default_version = self.default_version.clone();
        let av = available_versions.clone();
        router = router.route(
            "/api/",
            any(move |uri: Uri| async move {
                versioning::handle_unversioned(
                    default_version.clone(),
                    av.clone(),
                    uri,
                    axum::extract::Path("".to_string()),
                )
                .await
            }),
        );

        // Mounts (non-primary).
        for m in mounts.into_iter() {
            let p = m.path;
            let mut r = m.router;
            // Streaming-safe tracing for mount routes.
            r = r.layer(axum::middleware::from_fn(
                |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| async move {
                let start = std::time::Instant::now();
                let method = req.method().clone();
                let path = req.uri().path().to_string();
                let resp: axum::response::Response = next.run(req).await;
                log::info!(
                    "api-mount {} {} -> {} ({:?})",
                    method,
                    path,
                    resp.status(),
                    start.elapsed()
                );
                resp
            },
            ));
            if let Some(cors) = self.cors.clone() {
                r = r.layer(cors);
            }
            if let Some(auth) = self.auth.clone() {
                r = auth.layer(r);
            }
            router = router.nest(&p, r);
        }

        // Optional MCP coexistence at the fixed `/mcp` path.
        if let Some(mut mcp_router) = self.mcp_router {
            mcp_router = mcp_router.layer(axum::middleware::from_fn(
                |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| async move {
                    let start = std::time::Instant::now();
                    let method = req.method().clone();
                    let path = req.uri().path().to_string();
                    let resp: axum::response::Response = next.run(req).await;
                    log::info!(
                        "mcp {} {} -> {} ({:?})",
                        method,
                        path,
                        resp.status(),
                        start.elapsed()
                    );
                    resp
                },
            ));
            if let Some(cors) = self.cors.clone() {
                mcp_router = mcp_router.layer(cors);
            }
            if let Some(auth) = self.auth.clone() {
                mcp_router = auth.layer(mcp_router);
            }
            router = router.merge(mcp_router);
        }

        // If health should be protected, apply auth/cors the same way as APIs.
        if self.protect_health {
            if let Some(cors) = self.cors.clone() {
                health_router = health_router.layer(cors);
            }
            if let Some(auth) = self.auth.clone() {
                health_router = auth.layer(health_router);
            }
        }
        router = router.merge(health_router);

        ApiServer { router, shutdown }
    }
}

fn normalize_mount_path(raw: &str) -> Result<String, String> {
    if raw.is_empty() {
        return Err("mount path must not be empty".to_string());
    }
    if !raw.starts_with('/') {
        return Err(format!("mount path must start with '/': '{raw}'"));
    }
    if raw != "/" && raw.ends_with('/') {
        return Err(format!("mount path must not have a trailing '/': '{raw}'"));
    }
    Ok(raw.to_string())
}

fn is_prefix_path(prefix: &str, path: &str) -> bool {
    if prefix == path {
        return true;
    }
    if prefix == "/" {
        return true;
    }
    path.starts_with(prefix) && path.as_bytes().get(prefix.len()) == Some(&b'/')
}

fn paths_collide(a: &str, b: &str) -> bool {
    is_prefix_path(a, b) || is_prefix_path(b, a)
}

#[derive(Debug)]
pub struct ApiServer {
    router: axum::Router,
    shutdown: CancellationToken,
}

impl ApiServer {
    pub fn into_router(self) -> axum::Router {
        self.router
    }

    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub async fn serve(self, addr: &str) -> anyhow::Result<()> {
        let listener = tokio::net::TcpListener::bind(addr).await?;

        let shutdown_token = self.shutdown.clone();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        // Flip readiness before initiating listener shutdown. This gives callers a window where
        // `/readyz` returns 503 while the server is still accepting connections.
        tokio::spawn(async move {
            wait_for_shutdown_signal().await;
            shutdown_token.cancel();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = shutdown_tx.send(());
        });

        axum::serve(listener, self.router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await?;
        Ok(())
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = sigint.recv() => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
