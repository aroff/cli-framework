//! Built-in HTTP API hosting with versioned routing (`/api/{version}/...`).
//!
//! This module is behind the `api-server` feature flag.

mod headers;
mod health;
#[cfg(feature = "api-swagger")]
mod swagger;
mod versioning;

use crate::parser::error_codes;
use crate::tower;
use axum::http::Uri;
use axum::routing::{any, get};
use axum::Router;
use chrono::{DateTime, Utc};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tower::Layer;

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
    /// App-supplied OpenAPI document (`api-swagger` feature only).
    /// `Some(value)` → serves `/api/{version}/openapi.json` and adds a Swagger UI entry.
    /// `None` → no spec endpoint, no switcher entry.
    #[cfg(feature = "api-swagger")]
    pub openapi: Option<serde_json::Value>,
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

#[derive(Clone)]
struct ReadinessCheckHolder(ReadinessCheck);

impl std::fmt::Debug for ReadinessCheckHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadinessCheck(..)")
    }
}

#[derive(Clone)]
struct AuthLayerHolder(tower::util::BoxCloneLayer<axum::Router>);

impl std::fmt::Debug for AuthLayerHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthLayer(..)")
    }
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

#[derive(Debug)]
pub struct ApiServerBuilder {
    versions: BTreeMap<ApiVersionName, ApiVersion>,
    mounts: Vec<(String, axum::Router)>,
    default_version: DefaultVersion,
    cors: Option<tower_http::cors::CorsLayer>,
    // Type-erased layer: MUST be clonable and applicable to the router.
    auth: Option<AuthLayerHolder>,
    readiness_check: ReadinessCheckHolder,
    protect_health: bool,
    reserved_prefixes: BTreeSet<String>,
    #[cfg(feature = "mcp-server")]
    mcp_router: Option<axum::Router>,
    root_fallback: Option<axum::Router>,
    health_version: Option<String>,
}

impl Default for ApiServerBuilder {
    fn default() -> Self {
        Self {
            versions: BTreeMap::new(),
            mounts: Vec::new(),
            default_version: DefaultVersion::None,
            cors: None,
            auth: None,
            readiness_check: ReadinessCheckHolder(Arc::new(|| {
                Box::pin(async {
                    ReadinessReport {
                        ready: true,
                        checks: BTreeMap::new(),
                    }
                })
            })),
            protect_health: false,
            reserved_prefixes: BTreeSet::new(),
            #[cfg(feature = "mcp-server")]
            mcp_router: None,
            root_fallback: None,
            health_version: None,
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
        self.mounts.push((path.to_string(), router));
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

    pub fn auth(mut self, layer: tower::util::BoxCloneLayer<axum::Router>) -> Self {
        self.auth = Some(AuthLayerHolder(layer));
        self
    }

    #[cfg(feature = "mcp-server")]
    pub fn mcp_router(mut self, router: axum::Router) -> Self {
        self.mcp_router = Some(router);
        self
    }

    pub fn readiness_check(mut self, check: ReadinessCheck) -> Self {
        self.readiness_check = ReadinessCheckHolder(check);
        self
    }

    pub fn protect_health(mut self, yes: bool) -> Self {
        self.protect_health = yes;
        self
    }

    /// Attach a router to handle requests not matched by any host, version, mount,
    /// MCP, or Swagger route. Intended for serving a SPA or static assets at the root.
    ///
    /// Calling this method more than once overwrites the previous value (take-last).
    /// This does NOT relax the rule that the primary API is served only via `version(...)`.
    pub fn root_fallback(mut self, router: axum::Router) -> Self {
        self.root_fallback = Some(router);
        self
    }

    /// Override the version string reported by `GET /healthz`.
    ///
    /// By default `/healthz` reports the framework's own crate version
    /// (`env!("CARGO_PKG_VERSION")`), which is fixed at cli-framework's compile
    /// time. Consumers that want `/healthz` to report THEIR version should call
    /// this with their own version (e.g. their `env!("CARGO_PKG_VERSION")`).
    ///
    /// Calling this method more than once overwrites the previous value (take-last).
    pub fn health_version(mut self, v: impl Into<String>) -> Self {
        self.health_version = Some(v.into());
        self
    }

    pub fn reserved_prefixes(mut self, prefixes: &[&str]) -> Self {
        for p in prefixes {
            let normalized = normalize_mount_path(p)
                .unwrap_or_else(|e| panic_config(error_codes::E_API_MOUNT_COLLISION, e));
            self.reserved_prefixes.insert(normalized);
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
        let mut mounts: Vec<(String, axum::Router)> = Vec::with_capacity(self.mounts.len());
        for (path, router) in self.mounts.into_iter() {
            let p = normalize_mount_path(&path)
                .unwrap_or_else(|e| panic_config(error_codes::E_API_MOUNT_COLLISION, e));
            mounts.push((p, router));
        }

        // The primary API must only be served via versioned routes, so never allow mounting at `/` or `/api`.
        for (path, _) in mounts.iter() {
            if path == "/" {
                panic_config(
                    error_codes::E_API_MOUNT_COLLISION,
                    "mount('/') is not allowed; use /api/{version}/... for primary APIs",
                );
            }
            if path == "/api" {
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
        for (path, _) in mounts.iter() {
            for fixed in fixed_paths.iter().chain(reserved_api_prefixes.iter()) {
                if paths_collide(path, fixed) {
                    panic_config(
                        error_codes::E_API_MOUNT_COLLISION,
                        format!(
                            "mount path '{}' collides with reserved path '{}'",
                            path, fixed
                        ),
                    );
                }
            }
            for vp in version_prefixes.iter() {
                if paths_collide(path, vp) {
                    panic_config(
                        error_codes::E_API_MOUNT_COLLISION,
                        format!(
                            "mount path '{}' collides with api version prefix '{}'",
                            path, vp
                        ),
                    );
                }
            }
            for fp in fixed_prefixes.iter() {
                if paths_collide(path, fp) {
                    panic_config(
                        error_codes::E_API_MOUNT_COLLISION,
                        format!(
                            "mount path '{}' collides with reserved host prefix '{}'",
                            path, fp
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
        let shutdown_readiness = Arc::new(AtomicBool::new(false));

        let available_versions: Vec<String> = validated_versions
            .keys()
            .map(|v| v.as_str().to_string())
            .collect();

        let health_state = health::HealthState {
            shutdown: shutdown.clone(),
            shutdown_readiness: Arc::clone(&shutdown_readiness),
            readiness_check: Arc::clone(&self.readiness_check.0),
            crate_version: self
                .health_version
                .clone()
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        };

        let mut router = Router::new();

        let healthz_router = Router::new()
            .route("/", get(health::healthz))
            .with_state(health_state.clone());
        let readyz_router = Router::new()
            .route("/", get(health::readyz))
            .with_state(health_state);

        // API root: /api/{version}/...
        let mut api_root = Router::new();
        for (_, v) in validated_versions.iter() {
            let hc = HeaderConfig {
                api_version: v.name.as_str().to_string(),
                sunset: v.deprecation.as_ref().map(|d| d.sunset),
                docs_url: v.deprecation.as_ref().and_then(|d| d.docs_url.clone()),
            };
            let vr = with_tracing(
                headers::apply_versioned_headers(v.router.clone(), hc),
                "api",
            );
            let prefix = format!("/{}", v.name.as_str());
            api_root =
                nest_with_cors_auth(api_root, &prefix, vr, self.cors.clone(), self.auth.as_ref());
        }

        // Unversioned /api/... catch-all (redirect to default or 404).
        let dv = self.default_version.clone();
        let av = available_versions.clone();
        api_root = api_root.route(
            "/",
            any(move |uri: Uri| {
                let dv = dv.clone();
                let av = av.clone();
                async move { versioning::handle_unversioned(dv, av, uri, "").await }
            }),
        );
        let dv = self.default_version.clone();
        let av = available_versions.clone();
        api_root = api_root.route(
            "/{*rest}",
            any(
                move |uri: Uri, axum::extract::Path(rest): axum::extract::Path<String>| {
                    let dv = dv.clone();
                    let av = av.clone();
                    async move { versioning::handle_unversioned(dv, av, uri, &rest).await }
                },
            ),
        );

        router = router.nest("/api", api_root);

        // `/api/` doesn't match the nested router at `/api`, so handle it explicitly.
        let dv = self.default_version.clone();
        let av = available_versions.clone();
        router = router.route(
            "/api/",
            any(move |uri: Uri| {
                let dv = dv.clone();
                let av = av.clone();
                async move { versioning::handle_unversioned(dv, av, uri, "").await }
            }),
        );

        // Mounts (non-primary).
        for (p, r) in mounts.into_iter() {
            let r = with_tracing(r, "api-mount");
            router = nest_with_cors_auth(router, &p, r, self.cors.clone(), self.auth.as_ref());
        }

        // Optional MCP coexistence at the fixed `/mcp` path.
        #[cfg(feature = "mcp-server")]
        if let Some(mcp_router) = self.mcp_router {
            let mcp_router = with_tracing(mcp_router, "mcp");
            if let Some(cors) = self.cors.clone() {
                let mcp_router = mcp_router.layer(cors);
                if let Some(auth) = self.auth.as_ref() {
                    let svc = auth.0.clone().layer(mcp_router);
                    router = router
                        .route_service("/mcp", svc.clone())
                        .route_service("/mcp/*rest", svc);
                } else {
                    router = router.merge(mcp_router);
                }
            } else if let Some(auth) = self.auth.as_ref() {
                let svc = auth.0.clone().layer(mcp_router);
                router = router
                    .route_service("/mcp", svc.clone())
                    .route_service("/mcp/*rest", svc);
            } else {
                router = router.merge(mcp_router);
            }
        }

        // Health/readiness — optionally protected by auth.
        let cors = self.cors.clone();
        let auth = self.auth.as_ref();
        if self.protect_health {
            router = nest_with_cors_auth(router, "/healthz", healthz_router, cors.clone(), auth);
            router = nest_with_cors_auth(router, "/readyz", readyz_router, cors, auth);
        } else {
            router = router.nest_service(
                "/healthz",
                healthz_router.into_service::<axum::body::Body>(),
            );
            router =
                router.nest_service("/readyz", readyz_router.into_service::<axum::body::Body>());
        }

        // Swagger UI and per-version OpenAPI spec routes (api-swagger feature).
        #[cfg(feature = "api-swagger")]
        {
            let swagger_versions: Vec<(String, String)> = validated_versions
                .iter()
                .filter_map(|(name, v)| {
                    v.openapi.as_ref().map(|doc| {
                        let json = swagger::patch_and_serialize(doc.clone(), name.as_str())
                            .unwrap_or_else(|e| {
                                panic_config(
                                    error_codes::E_API_SWAGGER_SERIALIZE,
                                    format!(
                                        "failed to serialize openapi doc for '{}': {}",
                                        name.as_str(),
                                        e
                                    ),
                                )
                            });
                        (name.as_str().to_string(), json)
                    })
                })
                .collect();

            if !swagger_versions.is_empty() {
                let primary = match &self.default_version {
                    DefaultVersion::Pinned(v)
                        if swagger_versions.iter().any(|(n, _)| n == v.as_str()) =>
                    {
                        v.as_str().to_string()
                    }
                    _ => swagger_versions[0].0.clone(),
                };
                let swagger_router =
                    swagger::build_swagger_router(swagger_versions.clone(), &primary);

                if let Some(auth) = self.auth.as_ref() {
                    let svc = auth.0.clone().layer(swagger_router);
                    router = router
                        .route_service("/api/docs", svc.clone())
                        .route_service("/api/docs/", svc.clone())
                        .route_service("/api/docs/{*rest}", svc.clone());
                    for (name, _) in &swagger_versions {
                        router = router
                            .route_service(&format!("/api/{}/openapi.json", name), svc.clone());
                    }
                } else {
                    router = router.merge(swagger_router);
                }
            }
        }

        // Root fallback for SPA / static assets — wired last so all host routes win.
        if let Some(fb) = self.root_fallback {
            let mut fb = with_tracing(fb, "api-fallback");
            if let Some(cors) = self.cors.clone() {
                fb = fb.layer(cors);
            }
            router = router.fallback_service(fb.into_service::<axum::body::Body>());
        }

        ApiServer {
            router,
            shutdown,
            shutdown_readiness,
        }
    }
}

/// Wrap a `Router` with a streaming-safe request/response log at `label`.
fn with_tracing(router: Router, label: &'static str) -> Router {
    router.layer(axum::middleware::from_fn(
        move |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| async move {
            let start = std::time::Instant::now();
            let method = req.method().clone();
            let path = req.uri().path().to_string();
            let resp: axum::response::Response = next.run(req).await;
            log::info!(
                "{} {} {} -> {} ({:?})",
                label,
                method,
                path,
                resp.status(),
                start.elapsed()
            );
            resp
        },
    ))
}

/// Apply optional CORS then optional auth, then nest `subrouter` into `root` at `path`.
fn nest_with_cors_auth(
    mut root: Router,
    path: &str,
    mut subrouter: Router,
    cors: Option<tower_http::cors::CorsLayer>,
    auth: Option<&AuthLayerHolder>,
) -> Router {
    if let Some(cors) = cors {
        subrouter = subrouter.layer(cors);
    }
    if let Some(auth) = auth {
        let svc = auth.0.clone().layer(subrouter);
        root = root.nest_service(path, svc);
    } else {
        root = root.nest_service(path, subrouter.into_service::<axum::body::Body>());
    }
    root
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
    shutdown_readiness: Arc<AtomicBool>,
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
        let shutdown_readiness = Arc::clone(&self.shutdown_readiness);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        // Flip readiness before initiating listener shutdown. This gives callers a window where
        // `/readyz` returns 503 while the server is still accepting connections.
        tokio::spawn(async move {
            wait_for_shutdown_signal().await;
            shutdown_readiness.store(true, Ordering::SeqCst);
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
