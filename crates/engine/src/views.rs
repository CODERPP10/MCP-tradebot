//! Response shapes for the read services — the §5 tool output schemas.
//!
//! These are deliberately thin projections of the `kite-client` models: drop
//! fields the agent has no use for, flatten the couple that matter, and derive
//! the two booleans (`tradable`, `at_circuit_limit`) the raw quote only implies.

use serde::Serialize;

use kite_client::{Holding, Order, PositionsData, Quote, SegmentMargins};

#[derive(Debug, Clone, Serialize)]
pub struct PositionRow {
    pub tradingsymbol: String,
    pub quantity: i64,
    pub average_price: f64,
    pub last_price: f64,
    pub pnl: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PositionsView {
    pub net: Vec<PositionRow>,
    pub day: Vec<PositionRow>,
}

impl From<PositionsData> for PositionsView {
    fn from(d: PositionsData) -> Self {
        let row = |p: kite_client::Position| PositionRow {
            tradingsymbol: p.tradingsymbol,
            quantity: p.quantity,
            average_price: p.average_price,
            last_price: p.last_price,
            pnl: p.pnl,
        };
        Self {
            net: d.net.into_iter().map(row).collect(),
            day: d.day.into_iter().map(row).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HoldingRow {
    pub tradingsymbol: String,
    pub quantity: i64,
    pub average_price: f64,
    pub last_price: f64,
    pub pnl: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HoldingsView {
    pub holdings: Vec<HoldingRow>,
}

impl From<Vec<Holding>> for HoldingsView {
    fn from(hs: Vec<Holding>) -> Self {
        Self {
            holdings: hs
                .into_iter()
                .map(|h| HoldingRow {
                    tradingsymbol: h.tradingsymbol,
                    quantity: h.quantity + h.t1_quantity,
                    average_price: h.average_price,
                    last_price: h.last_price,
                    pnl: h.pnl,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MarginsView {
    /// Cash available to deploy (Kite `available.live_balance`).
    pub available: f64,
    /// Margin currently used (Kite `utilised.debits`).
    pub utilised: f64,
    pub net: f64,
}

impl From<SegmentMargins> for MarginsView {
    fn from(m: SegmentMargins) -> Self {
        Self {
            available: m.available.live_balance,
            utilised: m.utilised.debits,
            net: m.net,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OhlcView {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuoteView {
    pub last_price: f64,
    pub ohlc: OhlcView,
    pub timestamp: Option<String>,
    /// Best-effort: false only when the last price is sitting on a circuit band.
    pub tradable: bool,
    pub at_circuit_limit: bool,
}

impl From<Quote> for QuoteView {
    fn from(q: Quote) -> Self {
        let at_circuit_limit = (q.lower_circuit_limit > 0.0
            && q.last_price <= q.lower_circuit_limit)
            || (q.upper_circuit_limit > 0.0 && q.last_price >= q.upper_circuit_limit);
        Self {
            last_price: q.last_price,
            ohlc: OhlcView {
                open: q.ohlc.open,
                high: q.ohlc.high,
                low: q.ohlc.low,
                close: q.ohlc.close,
            },
            timestamp: q.timestamp,
            tradable: !at_circuit_limit,
            at_circuit_limit,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderRow {
    pub order_id: String,
    pub tradingsymbol: String,
    pub side: String,
    pub quantity: i64,
    pub filled_quantity: i64,
    pub limit_price: f64,
    pub status: String,
    /// The `tradebot:<ref_id>` tag if this order was placed by us, else null.
    pub ref_id: Option<String>,
}

impl OrderRow {
    fn from_order(o: Order) -> Self {
        let ref_id = o
            .tag
            .as_deref()
            .and_then(|t| t.strip_prefix("tradebot:"))
            .map(str::to_string);
        Self {
            order_id: o.order_id,
            tradingsymbol: o.tradingsymbol,
            side: o.transaction_type,
            quantity: o.quantity,
            filled_quantity: o.filled_quantity,
            limit_price: o.price,
            status: o.status,
            ref_id,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OrdersView {
    pub orders: Vec<OrderRow>,
}

impl From<Vec<Order>> for OrdersView {
    fn from(os: Vec<Order>) -> Self {
        Self {
            orders: os.into_iter().map(OrderRow::from_order).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderEvent {
    pub status: String,
    pub timestamp: Option<String>,
    pub filled_quantity: i64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderStatusView {
    pub status: String,
    pub filled_quantity: i64,
    pub pending_quantity: i64,
    pub average_price: f64,
    pub status_message: Option<String>,
    pub history: Vec<OrderEvent>,
}

impl OrderStatusView {
    /// `history` is the ordered list Kite returns from `GET /orders/:id`; the
    /// last row is the current state.
    pub(crate) fn from_history(history: Vec<Order>) -> Option<Self> {
        let current = history.last()?.clone();
        Some(Self {
            status: current.status,
            filled_quantity: current.filled_quantity,
            pending_quantity: current.pending_quantity,
            average_price: current.average_price,
            status_message: current.status_message,
            history: history
                .into_iter()
                .map(|o| OrderEvent {
                    status: o.status,
                    timestamp: o.order_timestamp.or(o.exchange_timestamp),
                    filled_quantity: o.filled_quantity,
                    message: o.status_message,
                })
                .collect(),
        })
    }
}
