//! Pure Kite Connect REST client (DESIGN.md §2.1). **No business logic.**
//!
//! - Auth header `Authorization: token <api_key>:<access_token>`.
//! - Session checksum `SHA256(api_key + request_token + api_secret)`.
//! - Client-side token-bucket rate limiter sized under Kite's published limits.
//! - Typed request/response models and a normalised [`KiteError`] taxonomy.
//! - **Never logs** secrets, tokens, or request/response bodies (no logging at all).
//! - **Never retries** a mutating call — that policy lives in the engine.
//!
//! Everything here is exercised against recorded HTTP fixtures (`wiremock`);
//! CI makes no live calls.

mod client;
mod error;
mod models;
mod rate_limit;

pub use client::{KiteClient, KiteConfig, DEFAULT_BASE_URL};
pub use error::KiteError;
pub use models::{
    AvailableMargin, Holding, Instrument, InstrumentParseError, LtpQuote, Ohlc, Order, OrderId,
    PlaceOrderRequest, Position, PositionsData, Profile, Quote, SegmentMargins, SessionData,
    UtilisedMargin,
};
pub use rate_limit::RateLimiter;
