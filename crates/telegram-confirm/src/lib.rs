//! Out-of-band human approval (DESIGN.md §2.7).
//!
//! Stub — implemented in `feat/telegram-confirm`. Long-polls the Telegram Bot
//! API, sends Approve/Reject inline buttons carrying a per-request nonce, accepts
//! callbacks only from the allowlisted chat id, and **fails closed** on timeout,
//! network failure, or an unreachable bot.

#[derive(Debug, thiserror::Error)]
pub enum ConfirmError {
    #[error("telegram-confirm is not implemented yet (feat/telegram-confirm)")]
    NotImplemented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Approved,
    Rejected,
    Timeout,
}
