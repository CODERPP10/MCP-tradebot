//! The MCP tool surface (DESIGN.md §2.8 / §5). Thin: each tool serializes an
//! `engine` read view straight to JSON text. No trading logic, no `kite-client`
//! use, and — in this milestone — no mutating tools at all.

use engine::{Engine, EngineError};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::Deserialize;

#[derive(Clone)]
pub struct Tradebot {
    engine: Engine,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolArgs {
    /// NSE equity trading symbol, e.g. `"INFY"`.
    pub tradingsymbol: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OrderIdArgs {
    /// Broker order id, as returned by `list_orders`.
    pub order_id: String,
}

#[tool_router]
impl Tradebot {
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }

    #[tool(
        description = "Net and intraday positions: tradingsymbol, quantity, average price, \
                          last price, and P&L."
    )]
    async fn get_positions(&self) -> Result<CallToolResult, McpError> {
        shape(self.engine.positions().await)
    }

    #[tool(
        description = "Long-term holdings (demat): tradingsymbol, quantity, average price, \
                          last price, and P&L."
    )]
    async fn get_holdings(&self) -> Result<CallToolResult, McpError> {
        shape(self.engine.holdings().await)
    }

    #[tool(
        description = "Equity-segment funds: cash available to deploy, margin utilised, \
                          and net balance (rupees)."
    )]
    async fn get_margins(&self) -> Result<CallToolResult, McpError> {
        shape(self.engine.margins().await)
    }

    #[tool(
        description = "Today's order book: every order with its status, fills, and — for \
                          orders this bot placed — its ref_id."
    )]
    async fn list_orders(&self) -> Result<CallToolResult, McpError> {
        shape(self.engine.orders().await)
    }

    #[tool(
        description = "Live quote for one NSE equity symbol: last price, OHLC, whether it \
                          is tradable and whether it is at a circuit limit."
    )]
    async fn get_quote(
        &self,
        Parameters(args): Parameters<SymbolArgs>,
    ) -> Result<CallToolResult, McpError> {
        shape(self.engine.quote(&args.tradingsymbol).await)
    }

    #[tool(description = "Status and fill history of one order by its broker order_id.")]
    async fn get_order_status(
        &self,
        Parameters(args): Parameters<OrderIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        shape(self.engine.order_status(&args.order_id).await)
    }
}

/// Serialize an engine read result to JSON tool output. `NeedsLogin` is a
/// normal (non-error) result the agent can act on; everything else is a tool
/// error.
fn shape<T: serde::Serialize>(r: Result<T, EngineError>) -> Result<CallToolResult, McpError> {
    match r {
        Ok(v) => {
            let json = serde_json::to_string_pretty(&v)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
        }
        Err(EngineError::NeedsLogin { login_url }) => {
            let body = serde_json::json!({
                "error": "NEEDS_LOGIN",
                "detail": "the daily Kite access token is missing or expired; \
                           the operator must run `tradebot login`",
                "login_url": login_url,
            });
            Ok(CallToolResult::success(vec![ContentBlock::text(
                body.to_string(),
            )]))
        }
        Err(e) => Err(McpError::internal_error(e.to_string(), None)),
    }
}

#[tool_handler]
impl ServerHandler for Tradebot {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("tradebot", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Read-only access to a personal Zerodha (Kite) account. All amounts are in \
                 rupees, quantities in whole shares. No order-placing tools are available in \
                 this build.",
            )
    }
}
