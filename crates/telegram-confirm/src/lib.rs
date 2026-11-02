//! Out-of-band human approval (DESIGN.md §2.7).
//!
//! Long-polls the Telegram Bot API, sends Approve/Reject inline buttons
//! carrying a per-request nonce, accepts callbacks only from the allowlisted
//! chat id, and **fails closed** on timeout, network failure, or an
//! unreachable bot.

mod client;
mod confirmer;
mod login;

pub use client::{CallbackQuery, Chat, Message, TelegramClient, TgUser, Update};
pub use confirmer::{
    format_order_summary, generate_nonce, is_expired, validate_callback, ConfirmResumer, Confirmer,
    LoginResumer,
};
pub use domain::ConfirmOutcome;
pub use login::extract_request_token;

#[derive(Debug, thiserror::Error)]
pub enum ConfirmError {
    #[error("telegram network error: {0}")]
    Network(String),
    #[error("telegram API error: {0}")]
    Api(String),
    #[error("ledger error: {0}")]
    Ledger(String),
}
