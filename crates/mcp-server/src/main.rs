//! `tradebot-mcp` — the MCP tool layer (DESIGN.md §2.8). `rmcp`, stdio.
//!
//! Milestone A: read-only. Six tools, all pass-throughs to `engine` read
//! services. No mutating tool is compiled in; `--live` is refused.

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
        /// Paper mode (default). Inert in this build — no mutating tools exist.
        #[arg(long)]
        paper: bool,
        /// Live order mode. Not available yet (Milestone B).
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
                    "--live is not available: this build has read-only tools only (Milestone A)"
                );
            }
            let engine = boot::build_engine()?;
            eprintln!("tradebot-mcp: serving 6 read-only tools over stdio");
            let service = Tradebot::new(engine).serve(stdio()).await?;
            service.waiting().await?;
            Ok(())
        }
    }
}
