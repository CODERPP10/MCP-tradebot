//! The MCP tool surface (DESIGN.md §2.8 / §5). Thin: each tool serializes an
//! `engine` read view straight to JSON text. No trading logic, no `kite-client`
//! use, and — in this milestone — no mutating tools at all.

use domain::{Exchange, Money, OrderIntent, OrderType, Product, Quantity, Side, Validity, Variety};
use engine::{Engine, EngineError, IntentSource, SubmitOutcome};
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PlaceEquityOrderArgs {
    /// NSE equity trading symbol, e.g. `"INFY"`.
    pub tradingsymbol: String,
    pub side: OrderSide,
    /// Whole shares, must be > 0.
    pub quantity: u32,
    /// Rupees. Must be within the policy's price band of the live LTP.
    pub limit_price: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[tool_router]
impl Tradebot {
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }

    #[tool(
        description = "Place an NSE equity CNC LIMIT order (DAY validity). Every order is \
                          evaluated by the guardrail policy engine first; most orders also \
                          require human approval via Telegram before anything reaches the \
                          broker. Returns immediately with status 'accepted' (paper-filled), \
                          'pending_confirmation' (poll get_order_status with ref_id), or \
                          'rejected' (see reasons)."
    )]
    async fn place_equity_order(
        &self,
        Parameters(args): Parameters<PlaceEquityOrderArgs>,
    ) -> Result<CallToolResult, McpError> {
        let quantity = match Quantity::new(args.quantity) {
            Ok(q) => q,
            Err(e) => {
                let body = serde_json::json!({"status": "rejected", "ref_id": null, "order_id": null,
                    "reasons": [{"code": "ORDER_SHAPE", "detail": e.to_string()}]});
                return Ok(CallToolResult::success(vec![ContentBlock::text(
                    body.to_string(),
                )]));
            }
        };
        let intent = OrderIntent {
            tradingsymbol: args.tradingsymbol,
            exchange: Exchange::Nse,
            side: match args.side {
                OrderSide::Buy => Side::Buy,
                OrderSide::Sell => Side::Sell,
            },
            quantity,
            product: Product::Cnc,
            variety: Variety::Regular,
            order_type: OrderType::Limit,
            validity: Validity::Day,
            limit_price: Some(Money::from_paise((args.limit_price * 100.0).round() as i64)),
        };
        match self.engine.submit(intent, IntentSource::Mcp).await {
            Ok(outcome) => Ok(CallToolResult::success(vec![ContentBlock::text(
                submit_outcome_json(outcome).to_string(),
            )])),
            Err(EngineError::NeedsLogin { login_url }) => {
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    serde_json::json!({"error": "NEEDS_LOGIN", "login_url": login_url}).to_string(),
                )]))
            }
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    #[tool(
        description = "Cancel an order by broker order_id. In paper mode, orders fill \
                          immediately, so this will always report 'rejected' — there is \
                          nothing open to cancel."
    )]
    async fn cancel_order(
        &self,
        Parameters(args): Parameters<OrderIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        match self.engine.cancel(&args.order_id).await {
            Ok(outcome) => Ok(CallToolResult::success(vec![ContentBlock::text(
                submit_outcome_json(outcome).to_string(),
            )])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
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

/// Shapes a `SubmitOutcome` into the DESIGN.md §5 `place_equity_order` /
/// `cancel_order` response schema.
fn submit_outcome_json(outcome: SubmitOutcome) -> serde_json::Value {
    match outcome {
        SubmitOutcome::Accepted { order_id, ref_id } => serde_json::json!({
            "status": "accepted", "ref_id": ref_id, "order_id": order_id, "reasons": [],
        }),
        SubmitOutcome::Pending { ref_id } => serde_json::json!({
            "status": "pending_confirmation", "ref_id": ref_id, "order_id": null, "reasons": [],
        }),
        SubmitOutcome::Rejected { ref_id, reasons } => serde_json::json!({
            "status": "rejected", "ref_id": ref_id, "order_id": null, "reasons": reasons,
        }),
    }
}

#[tool_handler]
impl ServerHandler for Tradebot {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("tradebot", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Access to a personal Zerodha (Kite) account, paper mode only. All amounts are \
                 in rupees, quantities in whole shares. Orders are NSE equity CNC LIMIT DAY \
                 only, evaluated by a guardrail policy engine and typically require human \
                 approval via Telegram before they fill (simulated in paper mode) — \
                 place_equity_order returns immediately; poll get_order_status or list_orders \
                 with the returned ref_id.",
            )
    }
}
