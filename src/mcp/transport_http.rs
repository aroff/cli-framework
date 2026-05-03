use crate::mcp::{CliFrameworkHandler, McpServerArgs, McpToolRegistry};
use anyhow::Result;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::sync::Arc;

pub async fn start_streamable_http(
    tool_registry: Arc<McpToolRegistry>,
    args: &McpServerArgs,
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

    // §4.8: Log in spec order after successful bind
    log::info!(
        "MCP server listening on http://{}:{}{}",
        args.host,
        args.port,
        args.path
    );
    log::info!("MCP: exported {} tools", tool_registry.tool_count());

    let session_manager = Arc::new(LocalSessionManager::default());
    let config = StreamableHttpServerConfig::default();

    let service = StreamableHttpService::new(
        {
            let tool_registry = Arc::clone(&tool_registry);
            move || Ok(CliFrameworkHandler::new(Arc::clone(&tool_registry)))
        },
        session_manager,
        config,
    );

    let path = args.path.clone();
    let router = axum::Router::new().nest_service(&path, service);

    axum::serve(listener, router)
        .await
        .map_err(|e| anyhow::anyhow!("MCP server error: {}", e))?;
    Ok(())
}
