//! Access-token lifecycle (DESIGN.md §2.3).
//!
//! Kite access tokens are invalidated at the start of each trading day
//! (~06:00 IST) and there is **no supported unattended login**. v1 impl
//! [`ManualPaste`] drives an interactive `tradebot login` each morning: it hands
//! out the Kite connect URL, exchanges the pasted `request_token` for an access
//! token via [`kite_client`], and seals the token (with issue / assumed-expiry
//! timestamps) into the [`secret_store`].
//!
//! Validity is a local best-effort estimate. A live `Auth` error from
//! `kite-client` is authoritative — the engine surfaces `NeedsLogin` regardless
//! of what this layer believes.

mod manual_paste;

pub use manual_paste::{ManualPaste, SealedToken};

use async_trait::async_trait;

/// The Kite Connect login endpoint. `?api_key=…&v=3` is appended.
pub const KITE_LOGIN_URL: &str = "https://kite.zerodha.com/connect/login";

/// India Standard Time is a fixed +05:30 offset (no DST).
pub const IST_OFFSET_SECS: i32 = 5 * 3600 + 30 * 60;

/// Returned when the caller must re-run the interactive login flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsLogin {
    pub login_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("secret store error: {0}")]
    Secret(String),
    #[error("kite login exchange failed: {0}")]
    Login(String),
    #[error("sealed access-token record is malformed: {0}")]
    Corrupt(String),
    #[error("clock is set before the epoch")]
    Clock,
}

/// The result of asking for the current access token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenStatus {
    /// A token believed to still be valid.
    Valid(String),
    /// No usable token — run the login flow at this URL.
    NeedsLogin(NeedsLogin),
}

/// Supplies a currently-valid Kite access token, or signals `NeedsLogin`.
///
/// v1 impl: [`ManualPaste`]. A future `ScheduledAutomated` (a separate OS task,
/// **not** agent-reachable, **not** in the MCP binary) slots in behind this
/// trait with zero downstream change.
#[async_trait]
pub trait TokenProvider: Send + Sync {
    async fn access_token(&self) -> Result<TokenStatus, SessionError>;
}
