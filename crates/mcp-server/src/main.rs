//! The MCP tool layer (DESIGN.md §2.8). Thin: schema in / schema out, no trading
//! logic, no direct `kite-client` use — every call goes through `engine`.
//!
//! Stub — implemented in `feat/mcp-server` (`rmcp`, stdio transport).

fn main() -> anyhow::Result<()> {
    anyhow::bail!("mcp-server is not implemented yet (feat/mcp-server)");
}
