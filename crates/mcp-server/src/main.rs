//! `tradebot-mcp` — the MCP tool layer (DESIGN.md §2.8). `rmcp`, stdio.
//!
//! Six read tools plus `place_equity_order`/`cancel_order`, paper mode only.
//! `--live` is refused — going live is a separate, later, deliberate change.

mod boot;
mod server;

use anyhow::Result;
use clap::{Parser, Subcommand};
use rmcp::transport::stdio;
use rmcp::ServiceExt;

use server::Tradebot;

#[derive(Parser)]
#[command(
    name = "tradebot-mcp",
    version,
    about = "MCP server exposing read-only Zerodha access to an LLM agent"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the MCP protocol over stdio (for Claude Desktop / MCP Inspector).
    Serve {
        /// Paper mode (default, and the only mode this build supports).
        #[arg(long)]
        paper: bool,
        /// Live order mode. Not available — deliberately refused.
        #[arg(long)]
        live: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Serve { live, paper: _ } => {
            if live {
                anyhow::bail!(
                    "--live is not available in this build: paper mode only, going live is a \
                     separate deliberate change"
                );
            }
            let boot = boot::build_engine().await?;
            if let Some(confirmer) = boot.confirmer.clone() {
                tokio::spawn(async move { confirmer.run().await });
            }
            eprintln!("tradebot-mcp: serving 8 tools over stdio (paper mode)");
            let service = Tradebot::new(boot.engine).serve(stdio()).await?;
            service.waiting().await?;
            Ok(())
        }
    }
}
