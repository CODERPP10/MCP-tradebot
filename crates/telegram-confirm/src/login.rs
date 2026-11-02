//! Automatic Kite re-login over Telegram: when the daily access token
//! expires, `Confirmer` (which already owns the one `getUpdates` poll loop
//! for this bot — see its module docs for why there can only be one)
//! proactively DMs the login URL to the allowlisted chat and completes the
//! login when the user replies with the `request_token` (either the bare
//! value or the whole pasted redirect URL).
//!
//! Decoupled from `session`/`kite-client` the same way order confirmation is
//! decoupled from `engine` — via a [`LoginResumer`] trait implemented at the
//! `mcp-server` boot layer, which has the concrete `ManualPaste`. This crate
//! only ever sees a `login_url: &str` and hands back a plain `request_token`.

use async_trait::async_trait;

/// Implemented by whatever owns `session::ManualPaste::complete_login` —
/// `mcp-server`'s boot wiring, in production.
#[async_trait]
pub trait LoginResumer: Send + Sync {
    /// Exchange `request_token` for a fresh access token and seal it.
    /// `Err` carries a short, user-facing reason (sent back to the chat).
    async fn complete_login(&self, request_token: &str) -> Result<(), String>;
}

/// `request_token` may be pasted as the bare value or as the full redirect
/// URL Kite sends the browser to — accept either.
pub fn extract_request_token(text: &str) -> String {
    let text = text.trim();
    match text.find("request_token=") {
        Some(idx) => {
            let rest = &text[idx + "request_token=".len()..];
            rest.split('&').next().unwrap_or(rest).trim().to_string()
        }
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_bare_token_unchanged() {
        assert_eq!(extract_request_token("  abc123  "), "abc123");
    }

    #[test]
    fn extracts_token_from_full_redirect_url() {
        assert_eq!(
            extract_request_token(
                "https://example.com/?request_token=abc123&action=login&status=success"
            ),
            "abc123"
        );
    }

    #[test]
    fn extracts_token_when_it_is_the_last_query_param() {
        assert_eq!(
            extract_request_token("https://example.com/?status=success&request_token=xyz"),
            "xyz"
        );
    }
}
