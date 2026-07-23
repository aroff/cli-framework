use crate::mcp::banner::{emit_banner, BannerData, BannerSettings};
use crate::mcp::resources::ResourceRegistry;
use crate::mcp::{CliFrameworkHandler, McpServerArgs, McpToolRegistry, McpTransportKind};
use anyhow::Result;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::sync::Arc;

/// Returns an `axum::Router` fragment with the MCP service mounted at the root
/// (`"/"` and `"/*path"`). The `path` parameter is **not** baked into the
/// returned router; it is kept for API compatibility and used by
/// [`start_streamable_http`] to nest the router at the correct URL prefix.
///
/// When using this with [`crate::api::ApiServerBuilder::mcp_router`], the
/// builder nests the returned router at `/mcp` automatically — callers should
/// pass any non-conflicting string (e.g. `"/mcp"`) and the builder handles
/// placement.
///
/// For standalone serving (not via `ApiServerBuilder`), use
/// [`start_streamable_http`] which wraps the router in the correct `nest`.
///
/// The returned router carries no middleware. TLS, auth, and rate-limiting are
/// the responsibility of the host application's outer router.
pub fn mcp_axum_router(tool_registry: Arc<McpToolRegistry>, path: &str) -> axum::Router {
    mcp_axum_router_with_resources(tool_registry, Arc::new(ResourceRegistry::new()), path)
}

/// Like [`mcp_axum_router`], but threads a populated [`ResourceRegistry`] into
/// the per-session handler so registered `ui://…` resources are served via
/// `resources/list` and `resources/read`.
///
/// See [`mcp_axum_router`] for path-prefix semantics.
pub fn mcp_axum_router_with_resources(
    tool_registry: Arc<McpToolRegistry>,
    resource_registry: Arc<ResourceRegistry>,
    _path: &str,
) -> axum::Router {
    build_router_with_config(
        tool_registry,
        resource_registry,
        StreamableHttpServerConfig::default(),
    )
}

/// Like [`mcp_axum_router_with_resources`], but overrides the inbound
/// `Host`-header allowlist that rmcp's Streamable HTTP transport enforces for
/// DNS-rebinding protection.
///
/// - `Some(hosts)` — allow exactly these authorities (host or `host:port`).
/// - `None`        — DISABLE Host validation entirely (accept any `Host`).
///
/// Passing `None` removes rmcp's DNS-rebinding guard and is appropriate ONLY
/// for network-isolated deployments (e.g. a cluster-internal listener gated by
/// NetworkPolicy, called server-to-server rather than from a browser). The
/// plain [`mcp_axum_router`] / [`mcp_axum_router_with_resources`] keep rmcp's
/// default (loopback-only) guard.
pub fn mcp_axum_router_with_host_policy(
    tool_registry: Arc<McpToolRegistry>,
    resource_registry: Arc<ResourceRegistry>,
    allowed_hosts: Option<Vec<String>>,
) -> axum::Router {
    let config = match allowed_hosts {
        Some(hosts) => StreamableHttpServerConfig::default().with_allowed_hosts(hosts),
        None => StreamableHttpServerConfig::default().disable_allowed_hosts(),
    };
    build_router_with_config(tool_registry, resource_registry, config)
}

/// Shared body behind [`mcp_axum_router_with_resources`] and
/// [`mcp_axum_router_with_host_policy`]: builds the `LocalSessionManager` +
/// `StreamableHttpService` (with the per-session `CliFrameworkHandler` closure)
/// and mounts it at the flat `"/"` + `"/{*path}"` routes. Only the
/// `StreamableHttpServerConfig` varies between callers.
fn build_router_with_config(
    tool_registry: Arc<McpToolRegistry>,
    resource_registry: Arc<ResourceRegistry>,
    config: StreamableHttpServerConfig,
) -> axum::Router {
    let session_manager = Arc::new(LocalSessionManager::default());
    let service = StreamableHttpService::new(
        {
            let tool_registry = Arc::clone(&tool_registry);
            let resource_registry = Arc::clone(&resource_registry);
            move || {
                Ok(
                    CliFrameworkHandler::new(Arc::clone(&tool_registry), McpTransportKind::Http)
                        .with_resource_registry(Arc::clone(&resource_registry)),
                )
            }
        },
        session_manager,
        config,
    );
    // Flat routes — no prefix baked in. Callers (ApiServerBuilder::mcp_router)
    // nest this at the desired path. StreamableHttpService is Clone.
    axum::Router::new()
        .route_service("/", service.clone())
        .route_service("/{*path}", service)
}

/// Refactored — delegates router construction to `mcp_axum_router`.
/// Signature and observable behavior are UNCHANGED.
pub async fn start_streamable_http(
    tool_registry: Arc<McpToolRegistry>,
    args: &McpServerArgs,
    banner: BannerSettings,
) -> Result<()> {
    start_streamable_http_with_resources(
        tool_registry,
        Arc::new(ResourceRegistry::new()),
        args,
        banner,
    )
    .await
}

/// Like [`start_streamable_http`], but threads a populated [`ResourceRegistry`]
/// into the served handler so registered `ui://…` resources are served.
pub async fn start_streamable_http_with_resources(
    tool_registry: Arc<McpToolRegistry>,
    resource_registry: Arc<ResourceRegistry>,
    args: &McpServerArgs,
    banner: BannerSettings,
) -> Result<()> {
    let addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
        anyhow::anyhow!(
            "MCP_BIND_FAILED: address {}:{} already in use: {}",
            args.host,
            args.port,
            e
        )
    })?;

    tracing::info!(
        "MCP server listening on http://{}:{}{}",
        args.host,
        args.port,
        args.path
    );
    tracing::info!("MCP: exported {} tools", tool_registry.tool_count());
    tracing::info!("MCP: exported {} resources", resource_registry.len());

    // Bind succeeded — print the startup banner (URL + tool list) to stdout.
    let data = BannerData::http(&args.host, args.port, &args.path, &tool_registry);
    emit_banner(&data, banner);

    // mcp_axum_router_with_resources returns a flat router (service at "/" and
    // "/*path"). Wrap it at the declared path prefix for standalone serving.
    let inner = mcp_axum_router_with_resources(tool_registry, resource_registry, &args.path);
    let router = axum::Router::new().nest(&args.path, inner);

    axum::serve(listener, router)
        .await
        .map_err(|e| anyhow::anyhow!("MCP server error: {}", e))?;
    Ok(())
}
