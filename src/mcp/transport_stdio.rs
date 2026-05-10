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
pub async fn start_stdio(tool_registry: Arc<McpToolRegistry>) -> Result<()> {
    log::info!("MCP stdio server starting (stdin/stdout)");
    log::info!("MCP: exported {} tools", tool_registry.tool_count());

    let serialize = Arc::new(Mutex::new(()));
    let handler = CliFrameworkHandler::new(Arc::clone(&tool_registry), McpTransportKind::Stdio)
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
            log::info!("MCP_STDIO_EOF: stdin closed; shutting down cleanly");
            Ok(())
        }
        other => {
            // Treat other quit reasons as clean shutdown unless rmcp surfaced an error above.
            log::info!("MCP stdio server stopped: {:?}", other);
            Ok(())
        }
    }
}
