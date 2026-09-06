//! Shared domain types for the tradebot workspace.
//!
//! This crate is a leaf: it has no dependency on any other workspace crate, so
//! every layer (guardrails, engine, mcp-server, cli) can agree on the same
//! representation of money, quantity, and order intent without a dependency
//! cycle.

use std::fmt;

use serde::{Deserialize, Serialize};

mod money;
pub use money::{Money, MoneyError};

/// Buy or sell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Buy,
    Sell,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Side::Buy => f.write_str("BUY"),
            Side::Sell => f.write_str("SELL"),
        }
    }
}

/// Whole-share order quantity. Always `>= 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Quantity(u32);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QuantityError {
    #[error("quantity must be at least 1")]
    Zero,
}

impl Quantity {
    pub fn new(shares: u32) -> Result<Self, QuantityError> {
        if shares == 0 {
            Err(QuantityError::Zero)
        } else {
            Ok(Self(shares))
        }
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<'de> Deserialize<'de> for Quantity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u32::deserialize(deserializer)?;
        Quantity::new(raw).map_err(serde::de::Error::custom)
    }
}

/// The order-shape dimensions the v1 guardrail allowlist pins (DESIGN.md §1).
/// Only `Nse` / `Cnc` / `Regular` / `Limit` / `Day` are accepted in v1; the
/// other variants exist so a rejected intent can still be represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Exchange {
    Nse,
    Bse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Product {
    Cnc,
    Mis,
    Nrml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Variety {
    Regular,
    Amo,
    Co,
    Bo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderType {
    Limit,
    Market,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Validity {
    Day,
    Ioc,
}

/// A schema-validated order request produced by the MCP tool layer or the CLI.
///
/// The LLM never produces anything richer than this — there is no "raw
/// passthrough" path to the broker (DESIGN.md §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderIntent {
    pub tradingsymbol: String,
    pub exchange: Exchange,
    pub side: Side,
    pub quantity: Quantity,
    pub product: Product,
    pub variety: Variety,
    pub order_type: OrderType,
    pub validity: Validity,
    /// Required for `OrderType::Limit`.
    pub limit_price: Option<Money>,
}

/// Machine-readable reason codes (mirrors DESIGN.md §5 "Error / rejection codes").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReasonCode {
    KillSwitch,
    InstrumentNotAllowed,
    InstrumentUnknown,
    OrderShape,
    PriceOutOfBand,
    QuoteStale,
    AtCircuitLimit,
    OrderValueCap,
    SymbolPositionCap,
    DailyOrderCount,
    DailyNotionalCap,
    DailyCapitalCap,
    InsufficientMargin,
    MarketClosed,
    RateLimited,
    DuplicateOrder,
    NeedsLogin,
    ConfirmTimeout,
    ConfirmRejected,
    BrokerRejected,
    ReconciliationDivergence,
    /// Placeholder while a check is not yet implemented — fail closed.
    NotImplemented,
}

/// A structured guardrail finding — never a bare bool (DESIGN.md §2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason {
    pub code: ReasonCode,
    pub detail: String,
}

impl Reason {
    pub fn new(code: ReasonCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// The outcome of `guardrails::evaluate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Decision {
    Approved,
    Rejected { reasons: Vec<Reason> },
    NeedsHumanConfirm { reasons: Vec<Reason> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantity_rejects_zero() {
        assert_eq!(Quantity::new(0), Err(QuantityError::Zero));
        assert_eq!(Quantity::new(5).unwrap().get(), 5);
    }

    #[test]
    fn side_display_matches_kite_wire_form() {
        assert_eq!(Side::Buy.to_string(), "BUY");
        assert_eq!(Side::Sell.to_string(), "SELL");
    }

    #[test]
    fn order_intent_roundtrips_through_json() {
        let intent = OrderIntent {
            tradingsymbol: "INFY".to_string(),
            exchange: Exchange::Nse,
            side: Side::Buy,
            quantity: Quantity::new(3).unwrap(),
            product: Product::Cnc,
            variety: Variety::Regular,
            order_type: OrderType::Limit,
            validity: Validity::Day,
            limit_price: Some(Money::from_rupees(1500)),
        };
        let json = serde_json::to_string(&intent).unwrap();
        let back: OrderIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(intent, back);
    }
}
