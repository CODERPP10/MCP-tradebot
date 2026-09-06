//! Typed request/response models for the Kite Connect REST API (DESIGN.md §2.1).
//!
//! These mirror Kite's wire shapes with **no business logic**. Prices arrive as
//! `f64` rupees exactly as Kite sends them; converting to [`domain::Money`] (and
//! deciding whether a sub-paise price is acceptable) is the engine's job.

use serde::{Deserialize, Serialize};

use domain::{Exchange, OrderType, Product, Side, Validity, Variety};

/// Kite's success envelope: `{ "status": "success", "data": <T> }`.
#[derive(Debug, Deserialize)]
pub(crate) struct Envelope<T> {
    pub data: T,
}

/// Result of `POST /session/token` (`generate_session`).
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SessionData {
    pub user_id: String,
    #[serde(default)]
    pub user_name: String,
    pub access_token: String,
    #[serde(default)]
    pub public_token: String,
    #[serde(default)]
    pub login_time: String,
}

/// Broker order id, echoed by `place_order` / `cancel_order` and used to key
/// reconciliation against the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrderId(pub String);

impl std::fmt::Display for OrderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct OrderIdData {
    pub order_id: String,
}

/// A validated equity order request, ready to be form-encoded for
/// `POST /orders/regular`. Built by the engine from a `domain::OrderIntent`;
/// the client does not construct or mutate it.
#[derive(Debug, Clone)]
pub struct PlaceOrderRequest {
    pub tradingsymbol: String,
    pub exchange: Exchange,
    pub transaction_type: Side,
    pub order_type: OrderType,
    pub quantity: u32,
    pub product: Product,
    pub validity: Validity,
    pub variety: Variety,
    /// Rupees. Required for `OrderType::Limit`.
    pub price: Option<f64>,
    /// Free-form broker tag, `<= 20` chars. The engine sets `tradebot:<ref_id>`.
    pub tag: Option<String>,
}

impl PlaceOrderRequest {
    /// Form parameters for the POST body. `variety` goes in the URL path, not here.
    pub(crate) fn form_params(&self) -> Vec<(&'static str, String)> {
        let mut p = vec![
            ("tradingsymbol", self.tradingsymbol.clone()),
            ("exchange", wire_exchange(self.exchange).to_string()),
            ("transaction_type", self.transaction_type.to_string()),
            ("order_type", wire_order_type(self.order_type).to_string()),
            ("quantity", self.quantity.to_string()),
            ("product", wire_product(self.product).to_string()),
            ("validity", wire_validity(self.validity).to_string()),
        ];
        if let Some(price) = self.price {
            p.push(("price", format!("{price:.2}")));
        }
        if let Some(tag) = &self.tag {
            p.push(("tag", tag.clone()));
        }
        p
    }
}

pub(crate) fn wire_exchange(e: Exchange) -> &'static str {
    match e {
        Exchange::Nse => "NSE",
        Exchange::Bse => "BSE",
    }
}
pub(crate) fn wire_product(p: Product) -> &'static str {
    match p {
        Product::Cnc => "CNC",
        Product::Mis => "MIS",
        Product::Nrml => "NRML",
    }
}
pub(crate) fn wire_order_type(o: OrderType) -> &'static str {
    match o {
        OrderType::Limit => "LIMIT",
        OrderType::Market => "MARKET",
    }
}
pub(crate) fn wire_validity(v: Validity) -> &'static str {
    match v {
        Validity::Day => "DAY",
        Validity::Ioc => "IOC",
    }
}
pub(crate) fn wire_variety(v: Variety) -> &'static str {
    match v {
        Variety::Regular => "regular",
        Variety::Amo => "amo",
        Variety::Co => "co",
        Variety::Bo => "bo",
    }
}

/// One row from `GET /orders` or `GET /orders/:id` (order history).
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Order {
    pub order_id: String,
    #[serde(default)]
    pub parent_order_id: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub status_message: Option<String>,
    #[serde(default)]
    pub tradingsymbol: String,
    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub transaction_type: String,
    #[serde(default)]
    pub order_type: String,
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    pub validity: String,
    #[serde(default)]
    pub quantity: i64,
    #[serde(default)]
    pub filled_quantity: i64,
    #[serde(default)]
    pub pending_quantity: i64,
    #[serde(default)]
    pub cancelled_quantity: i64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub average_price: f64,
    #[serde(default)]
    pub trigger_price: f64,
    #[serde(default)]
    pub order_timestamp: Option<String>,
    #[serde(default)]
    pub exchange_timestamp: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
}

/// `GET /portfolio/positions` → `{ net: [...], day: [...] }`.
#[derive(Debug, Clone, Deserialize)]
pub struct PositionsData {
    pub net: Vec<Position>,
    pub day: Vec<Position>,
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Position {
    pub tradingsymbol: String,
    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    pub quantity: i64,
    #[serde(default)]
    pub average_price: f64,
    #[serde(default)]
    pub last_price: f64,
    #[serde(default)]
    pub pnl: f64,
    #[serde(default)]
    pub buy_quantity: i64,
    #[serde(default)]
    pub sell_quantity: i64,
}

/// One row from `GET /portfolio/holdings`.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Holding {
    pub tradingsymbol: String,
    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub isin: String,
    #[serde(default)]
    pub quantity: i64,
    #[serde(default)]
    pub t1_quantity: i64,
    #[serde(default)]
    pub average_price: f64,
    #[serde(default)]
    pub last_price: f64,
    #[serde(default)]
    pub pnl: f64,
}

/// `GET /user/margins/equity` — the equity segment only (v1 is equity-only).
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SegmentMargins {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub net: f64,
    pub available: AvailableMargin,
    pub utilised: UtilisedMargin,
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct AvailableMargin {
    #[serde(default)]
    pub live_balance: f64,
    #[serde(default)]
    pub cash: f64,
    #[serde(default)]
    pub opening_balance: f64,
    #[serde(default)]
    pub intraday_payin: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct UtilisedMargin {
    #[serde(default)]
    pub debits: f64,
    #[serde(default)]
    pub exposure: f64,
    #[serde(default)]
    pub m2m_realised: f64,
    #[serde(default)]
    pub m2m_unrealised: f64,
}

/// Full quote for one instrument from `GET /quote`.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Quote {
    pub instrument_token: u64,
    #[serde(default)]
    pub last_price: f64,
    #[serde(default)]
    pub last_trade_time: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub lower_circuit_limit: f64,
    #[serde(default)]
    pub upper_circuit_limit: f64,
    #[serde(default)]
    pub ohlc: Ohlc,
    #[serde(default)]
    pub volume: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Ohlc {
    #[serde(default)]
    pub open: f64,
    #[serde(default)]
    pub high: f64,
    #[serde(default)]
    pub low: f64,
    #[serde(default)]
    pub close: f64,
}

/// Last-price-only entry from `GET /quote/ltp`.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct LtpQuote {
    pub instrument_token: u64,
    pub last_price: f64,
}

/// `GET /user/profile`.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Profile {
    pub user_id: String,
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub broker: String,
}

/// One instrument from the daily `GET /instruments` CSV dump. Only the fields
/// the guardrail instrument check needs are retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instrument {
    pub instrument_token: u64,
    pub exchange_token: u64,
    pub tradingsymbol: String,
    pub name: String,
    pub segment: String,
    pub exchange: String,
    pub instrument_type: String,
}

impl Instrument {
    /// Parse the instrument dump CSV. Kite's header is:
    /// `instrument_token,exchange_token,tradingsymbol,name,last_price,expiry,
    /// strike,tick_size,lot_size,instrument_type,segment,exchange`
    pub fn parse_csv(body: &str) -> Result<Vec<Instrument>, InstrumentParseError> {
        let mut lines = body.lines();
        let header = lines.next().ok_or(InstrumentParseError::Empty)?;
        let cols: Vec<&str> = header.split(',').map(str::trim).collect();
        let idx = |name: &str| {
            cols.iter()
                .position(|c| *c == name)
                .ok_or_else(|| InstrumentParseError::MissingColumn(name.to_string()))
        };
        let (i_tok, i_extok, i_sym, i_name, i_type, i_seg, i_exch) = (
            idx("instrument_token")?,
            idx("exchange_token")?,
            idx("tradingsymbol")?,
            idx("name")?,
            idx("instrument_type")?,
            idx("segment")?,
            idx("exchange")?,
        );
        let max_idx = [i_tok, i_extok, i_sym, i_name, i_type, i_seg, i_exch]
            .into_iter()
            .max()
            .unwrap();

        let mut out = Vec::new();
        for (n, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split(',').collect();
            if f.len() <= max_idx {
                return Err(InstrumentParseError::ShortRow { line: n + 2 });
            }
            out.push(Instrument {
                instrument_token: f[i_tok]
                    .trim()
                    .parse()
                    .map_err(|_| InstrumentParseError::BadNumber { line: n + 2 })?,
                exchange_token: f[i_extok]
                    .trim()
                    .parse()
                    .map_err(|_| InstrumentParseError::BadNumber { line: n + 2 })?,
                tradingsymbol: f[i_sym].trim().to_string(),
                name: f[i_name].trim().trim_matches('"').to_string(),
                instrument_type: f[i_type].trim().to_string(),
                segment: f[i_seg].trim().to_string(),
                exchange: f[i_exch].trim().to_string(),
            });
        }
        Ok(out)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InstrumentParseError {
    #[error("instrument dump was empty")]
    Empty,
    #[error("instrument dump missing column `{0}`")]
    MissingColumn(String),
    #[error("instrument dump row {line} has too few fields")]
    ShortRow { line: usize },
    #[error("instrument dump row {line} has an unparseable numeric field")]
    BadNumber { line: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Exchange, OrderType, Product, Side, Validity, Variety};

    #[test]
    fn place_order_form_params_are_kite_wire_form() {
        let req = PlaceOrderRequest {
            tradingsymbol: "INFY".into(),
            exchange: Exchange::Nse,
            transaction_type: Side::Buy,
            order_type: OrderType::Limit,
            quantity: 3,
            product: Product::Cnc,
            validity: Validity::Day,
            variety: Variety::Regular,
            price: Some(1500.5),
            tag: Some("tradebot:01H".into()),
        };
        let p = req.form_params();
        assert!(p.contains(&("exchange", "NSE".to_string())));
        assert!(p.contains(&("transaction_type", "BUY".to_string())));
        assert!(p.contains(&("order_type", "LIMIT".to_string())));
        assert!(p.contains(&("product", "CNC".to_string())));
        assert!(p.contains(&("validity", "DAY".to_string())));
        assert!(p.contains(&("price", "1500.50".to_string())));
        assert!(p.contains(&("tag", "tradebot:01H".to_string())));
        assert_eq!(wire_variety(req.variety), "regular");
    }

    #[test]
    fn market_order_omits_price() {
        let req = PlaceOrderRequest {
            tradingsymbol: "INFY".into(),
            exchange: Exchange::Nse,
            transaction_type: Side::Sell,
            order_type: OrderType::Market,
            quantity: 1,
            product: Product::Cnc,
            validity: Validity::Day,
            variety: Variety::Regular,
            price: None,
            tag: None,
        };
        let p = req.form_params();
        assert!(!p.iter().any(|(k, _)| *k == "price"));
        assert!(!p.iter().any(|(k, _)| *k == "tag"));
    }

    #[test]
    fn parse_instruments_csv_keeps_needed_columns() {
        let csv = "instrument_token,exchange_token,tradingsymbol,name,last_price,expiry,strike,tick_size,lot_size,instrument_type,segment,exchange\n\
                   408065,1594,INFY,\"INFOSYS\",0,,0,0.05,1,EQ,NSE,NSE\n\
                   2953217,11536,TCS,\"TATA CONSULTANCY\",0,,0,0.05,1,EQ,NSE,NSE\n";
        let got = Instrument::parse_csv(csv).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].tradingsymbol, "INFY");
        assert_eq!(got[0].name, "INFOSYS");
        assert_eq!(got[0].instrument_token, 408065);
        assert_eq!(got[0].instrument_type, "EQ");
        assert_eq!(got[1].tradingsymbol, "TCS");
    }

    #[test]
    fn parse_instruments_csv_rejects_missing_column() {
        let csv = "instrument_token,tradingsymbol\n1,INFY\n";
        assert_eq!(
            Instrument::parse_csv(csv),
            Err(InstrumentParseError::MissingColumn("exchange_token".into()))
        );
    }
}
