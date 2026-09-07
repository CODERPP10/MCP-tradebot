//! Normalised error taxonomy for every Kite call (DESIGN.md §2.1).
//!
//! Kite returns a JSON envelope `{ "status": "error", "message": ..., "error_type": ... }`
//! alongside an HTTP status. We collapse both into a small closed set so callers
//! never branch on Kite's ~11 `error_type` strings or raw status codes.

use serde::Deserialize;

/// Every fallible `KiteClient` method returns this.
#[derive(Debug, thiserror::Error)]
pub enum KiteError {
    /// Access token missing, expired, or rejected; or a permission error.
    /// The session layer treats this as authoritative and surfaces `NeedsLogin`.
    #[error("authentication failed or token expired")]
    Auth,
    /// Kite's `TooManyRequestsException`, or a local rate-limiter rejection.
    #[error("rate limited")]
    RateLimited,
    /// The request did not complete: connect/read timeout, TLS failure, or
    /// Kite's own `NetworkException` (broker↔exchange link).
    #[error("network timeout or connection failure")]
    NetworkTimeout,
    /// The broker understood the request and refused it (bad input, order
    /// rejected, insufficient margin, …). Terminal — never retried.
    #[error("rejected by broker: [{code}] {message}")]
    Rejected { code: String, message: String },
    /// Anything we could not classify: unexpected status, unparseable body,
    /// an `error_type` we don't map. Fail closed.
    #[error("unknown Kite error: {0}")]
    Unknown(String),
}

impl KiteError {
    /// Map Kite's error envelope (already parsed) plus the HTTP status into the
    /// taxonomy. `message` is Kite's human string; it may be surfaced to the
    /// operator but is never logged with request context.
    pub(crate) fn from_envelope(status: u16, error_type: Option<&str>, message: &str) -> Self {
        match error_type {
            Some("TokenException" | "PermissionException" | "UserException") => KiteError::Auth,
            Some("TooManyRequestsException") => KiteError::RateLimited,
            Some("NetworkException") => KiteError::NetworkTimeout,
            Some(
                code @ ("OrderException" | "InputException" | "MarginException"
                | "HoldingException"),
            ) => KiteError::Rejected {
                code: code.to_string(),
                message: message.to_string(),
            },
            Some(other) => {
                KiteError::Unknown(format!("error_type={other} status={status}: {message}"))
            }
            None => match status {
                401 | 403 => KiteError::Auth,
                429 => KiteError::RateLimited,
                408 | 504 => KiteError::NetworkTimeout,
                400 | 422 => KiteError::Rejected {
                    code: format!("HTTP{status}"),
                    message: message.to_string(),
                },
                _ => KiteError::Unknown(format!("status={status}: {message}")),
            },
        }
    }
}

impl From<reqwest::Error> for KiteError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() || e.is_connect() {
            KiteError::NetworkTimeout
        } else if e.is_decode() {
            KiteError::Unknown(format!("response decode failed: {e}"))
        } else {
            KiteError::Unknown(e.to_string())
        }
    }
}

/// Kite's error-response shape. Success responses use [`Envelope`](crate::models::Envelope).
#[derive(Debug, Deserialize)]
pub(crate) struct ErrorBody {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub error_type: Option<String>,
}
