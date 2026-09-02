//! Access-token lifecycle (DESIGN.md §2.3).
//!
//! Stub — implemented in `feat/session-auth`. Kite tokens are invalidated at the
//! start of each trading day (~06:00 IST) and there is no supported unattended
//! login, so v1 impl `ManualPaste` drives `tradebot login` each morning.

/// Returned when the caller must re-run the interactive login flow.
#[derive(Debug, Clone)]
pub struct NeedsLogin {
    pub login_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("token provider is not implemented yet (feat/session-auth)")]
    NotImplemented,
}

/// Supplies a currently-valid Kite access token, or signals `NeedsLogin`.
///
/// v1 impl: `ManualPaste`. Future impl: `ScheduledAutomated` (separate OS task,
/// not agent-reachable) slots in with zero downstream change.
pub trait TokenProvider {
    fn access_token(&self) -> Result<Result<String, NeedsLogin>, SessionError>;
}
