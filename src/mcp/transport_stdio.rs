use crate::mcp::banner::{emit_banner, BannerData, BannerSettings};
use crate::mcp::resources::ResourceRegistry;
use crate::mcp::{CliFrameworkHandler, McpToolRegistry, McpTransportKind};
use anyhow::Result;
use rmcp::{serve_server, transport::stdio};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Start an MCP server over stdio (JSON-RPC over stdin/stdout).
///
/// Notes:
/// - Stdout is reserved for the JSON-RPC transport channel; hosts/commands MUST NOT print to stdout.
/// - Tool calls are serialized behind a mutex to reduce the chance of interleaved stdout writes.
pub async fn start_stdio(
    tool_registry: Arc<McpToolRegistry>,
    banner: BannerSettings,
) -> Result<()> {
    start_stdio_with_resources(tool_registry, Arc::new(ResourceRegistry::new()), banner).await
}

/// Like [`start_stdio`], but threads a populated [`ResourceRegistry`] into the
/// served handler so registered `ui://…` resources are served via
/// `resources/list` and `resources/read`.
pub async fn start_stdio_with_resources(
    tool_registry: Arc<McpToolRegistry>,
    resource_registry: Arc<ResourceRegistry>,
    banner: BannerSettings,
) -> Result<()> {
    tracing::info!("MCP stdio server starting (stdin/stdout)");
    tracing::info!("MCP: exported {} tools", tool_registry.tool_count());
    tracing::info!("MCP: exported {} resources", resource_registry.len());

    // Banner goes to stderr — stdout is the JSON-RPC channel.
    let data = BannerData::stdio(&tool_registry);
    emit_banner(&data, banner);

    let serialize = Arc::new(Mutex::new(()));
    let handler = CliFrameworkHandler::new(Arc::clone(&tool_registry), McpTransportKind::Stdio)
        .with_resource_registry(resource_registry)
        .with_stdio_serialization(serialize);

    let running = serve_server(handler, stdio())
        .await
        .map_err(|e| anyhow::anyhow!("MCP_STDIO_IO_ERROR: {}", e))?;

    match running
        .waiting()
        .await
        .map_err(|e| anyhow::anyhow!("MCP_STDIO_IO_ERROR: {}", e))?
    {
        rmcp::service::QuitReason::Closed => {
            tracing::info!("MCP_STDIO_EOF: stdin closed; shutting down cleanly");
            Ok(())
        }
        other => {
            // Treat other quit reasons as clean shutdown unless rmcp surfaced an error above.
            tracing::info!("MCP stdio server stopped: {:?}", other);
            Ok(())
        }
    }
}
