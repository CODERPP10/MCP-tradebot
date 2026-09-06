//! Pure Kite Connect REST client (DESIGN.md §2.1). **No business logic.**
//!
//! Stub — implemented in `feat/kite-client`. Will provide typed request/response
//! models, a client-side token-bucket rate limiter, and the normalised error
//! taxonomy below. Never logs secrets, tokens, or full request bodies.

/// Normalised error taxonomy for every Kite call (DESIGN.md §2.1).
#[derive(Debug, thiserror::Error)]
pub enum KiteError {
    #[error("authentication failed or token expired")]
    Auth,
    #[error("rate limited by Kite")]
    RateLimited,
    #[error("network timeout")]
    NetworkTimeout,
    #[error("order rejected by broker: {code} {message}")]
    Rejected { code: String, message: String },
    #[error("unknown Kite error: {0}")]
    Unknown(String),
}
