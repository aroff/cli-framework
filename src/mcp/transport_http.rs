use crate::mcp::banner::{emit_banner, BannerData, BannerSettings};
use crate::mcp::{CliFrameworkHandler, McpServerArgs, McpToolRegistry, McpTransportKind};
use anyhow::Result;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::sync::Arc;

/// Returns an `axum::Router` fragment that serves the MCP Streamable HTTP protocol
/// under `path`. Does NOT bind any port; the caller owns the `TcpListener` and
/// must call `axum::serve` themselves.
///
/// # Path prefix
/// `path` is forwarded verbatim to `nest_service`. The conventional value is `"/mcp"`.
/// The caller is responsible for preventing prefix collisions with other routes.
///
/// # Middleware
/// The returned router carries no middleware. TLS, auth, and rate-limiting are
/// the responsibility of the host application's outer router.
pub fn mcp_axum_router(tool_registry: Arc<McpToolRegistry>, path: &str) -> axum::Router {
    let session_manager = Arc::new(LocalSessionManager::default());
    let config = StreamableHttpServerConfig::default();
    let service = StreamableHttpService::new(
        {
            let tool_registry = Arc::clone(&tool_registry);
            move || {
                Ok(CliFrameworkHandler::new(
                    Arc::clone(&tool_registry),
                    McpTransportKind::Http,
                ))
            }
        },
        session_manager,
        config,
    );
    axum::Router::new().nest_service(path, service)
}

/// Refactored — delegates router construction to `mcp_axum_router`.
/// Signature and observable behavior are UNCHANGED.
pub async fn start_streamable_http(
    tool_registry: Arc<McpToolRegistry>,
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

    // Bind succeeded — print the startup banner (URL + tool list) to stdout.
    let data = BannerData::http(&args.host, args.port, &args.path, &tool_registry);
    emit_banner(&data, banner);

    let router = mcp_axum_router(tool_registry, &args.path);

    axum::serve(listener, router)
        .await
        .map_err(|e| anyhow::anyhow!("MCP server error: {}", e))?;
    Ok(())
}
