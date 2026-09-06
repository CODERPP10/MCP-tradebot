//! The single choke point (DESIGN.md §2.6).
//!
//! `submit()` (mutating) is still a stub — it lands with `feat/guardrails` +
//! `feat/ledger`. What is implemented here is the **read side** (§2.6 "Read
//! services"): [`Engine`] resolves the sealed access token via [`session`],
//! installs it on the [`KiteClient`], calls a single read endpoint, and shapes
//! the result into the §5 response views. No mutating Kite method is reachable
//! from this path.

use std::sync::Arc;

use domain::OrderIntent;
use kite_client::KiteClient;
use session::{TokenProvider, TokenStatus};

mod views;
pub use views::{
    HoldingRow, HoldingsView, MarginsView, OhlcView, OrderEvent, OrderRow, OrderStatusView,
    OrdersView, PositionRow, PositionsView, QuoteView,
};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// No usable access token — the caller must run the interactive login.
    #[error("needs login: {login_url}")]
    NeedsLogin { login_url: String },
    /// A Kite call failed. String, not the `KiteError`, to keep the taxonomy
    /// from leaking through the public API before it is shaped for tools.
    #[error("kite call failed: {0}")]
    Kite(String),
    /// A read succeeded but returned nothing for the requested key.
    #[error("no data for {0}")]
    NotFound(String),
    #[error("engine submit is not implemented yet (feat/guardrails)")]
    NotImplemented,
}

#[derive(Debug, Clone)]
pub enum SubmitOutcome {
    Accepted {
        order_id: String,
        ref_id: String,
    },
    Pending {
        ref_id: String,
    },
    Rejected {
        ref_id: String,
        reasons: Vec<domain::Reason>,
    },
}

/// Orchestration surface. Cheap to clone (shares the Kite client + token provider).
#[derive(Clone)]
pub struct Engine {
    kite: KiteClient,
    tokens: Arc<dyn TokenProvider>,
    /// `EXCH:` prefix for bare symbols. v1 is NSE-equity only.
    exchange: &'static str,
}

impl Engine {
    pub fn new(kite: KiteClient, tokens: Arc<dyn TokenProvider>) -> Self {
        Self {
            kite,
            tokens,
            exchange: "NSE",
        }
    }

    /// Resolve the sealed token and install it on the Kite client, or return
    /// `NeedsLogin`. Every read call funnels through this first.
    async fn authenticate(&self) -> Result<(), EngineError> {
        match self
            .tokens
            .access_token()
            .await
            .map_err(|e| EngineError::Kite(e.to_string()))?
        {
            TokenStatus::Valid(tok) => {
                self.kite.set_access_token(tok).await;
                Ok(())
            }
            TokenStatus::NeedsLogin(n) => Err(EngineError::NeedsLogin {
                login_url: n.login_url,
            }),
        }
    }

    // ---- read services (§2.6) -------------------------------------------

    pub async fn positions(&self) -> Result<PositionsView, EngineError> {
        self.authenticate().await?;
        let d = self.kite.positions().await.map_err(kite_err)?;
        Ok(PositionsView::from(d))
    }

    pub async fn holdings(&self) -> Result<HoldingsView, EngineError> {
        self.authenticate().await?;
        let h = self.kite.holdings().await.map_err(kite_err)?;
        Ok(HoldingsView::from(h))
    }

    pub async fn margins(&self) -> Result<MarginsView, EngineError> {
        self.authenticate().await?;
        let m = self.kite.margins_equity().await.map_err(kite_err)?;
        Ok(MarginsView::from(m))
    }

    pub async fn orders(&self) -> Result<OrdersView, EngineError> {
        self.authenticate().await?;
        let o = self.kite.orders().await.map_err(kite_err)?;
        Ok(OrdersView::from(o))
    }

    /// `tradingsymbol` is a bare NSE symbol, e.g. `"INFY"`.
    pub async fn quote(&self, tradingsymbol: &str) -> Result<QuoteView, EngineError> {
        self.authenticate().await?;
        let key = format!("{}:{}", self.exchange, tradingsymbol.trim().to_uppercase());
        let map = self.kite.quote(&[key.as_str()]).await.map_err(kite_err)?;
        map.into_iter()
            .next()
            .map(|(_, q)| QuoteView::from(q))
            .ok_or(EngineError::NotFound(key))
    }

    pub async fn order_status(&self, order_id: &str) -> Result<OrderStatusView, EngineError> {
        self.authenticate().await?;
        let history = self
            .kite
            .order_history(order_id.trim())
            .await
            .map_err(kite_err)?;
        OrderStatusView::from_history(history)
            .ok_or_else(|| EngineError::NotFound(order_id.to_string()))
    }
}

fn kite_err(e: kite_client::KiteError) -> EngineError {
    EngineError::Kite(e.to_string())
}

pub async fn submit(_intent: OrderIntent) -> Result<SubmitOutcome, EngineError> {
    Err(EngineError::NotImplemented)
}
