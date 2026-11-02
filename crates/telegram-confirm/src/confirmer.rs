//! Out-of-band human approval (DESIGN.md §2.7). Polls `Ledger` for new
//! `pending_confirm` rows, sends each one to the allowlisted Telegram chat
//! with Approve/Reject buttons, and resolves it — via [`ConfirmResumer`], not
//! a direct dependency on `engine` (see `domain::ConfirmOutcome` for why).
//!
//! Also owns the automatic-relogin flow (see the `login` module):
//! `getUpdates(offset, ...)` advances a single shared server-side queue for
//! the whole bot, so there can only be **one** poll loop per bot token — a
//! second independent poller (even for an unrelated feature) would
//! nondeterministically steal updates from this one. Login handling is
//! therefore folded into this same `poll_once`/`run`, not a separate struct.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Utc};
use domain::ConfirmOutcome;
use ledger::{Ledger, PendingConfirm};

use crate::client::{CallbackQuery, Message, TelegramClient};
use crate::login::extract_request_token;
pub use crate::login::LoginResumer;
use crate::ConfirmError;

/// Implemented by whatever owns `engine::resume_confirmed` — `mcp-server`'s
/// boot wiring, in production. Kept as a trait so `telegram-confirm` never
/// depends on `engine` (would create a dependency cycle: `engine` writes the
/// `pending_confirm` row this crate polls for).
#[async_trait]
pub trait ConfirmResumer: Send + Sync {
    async fn resume(&self, ref_id: &str, outcome: ConfirmOutcome);
}

pub struct Confirmer<R: ConfirmResumer> {
    client: TelegramClient,
    ledger: Arc<Ledger>,
    chat_id: String,
    ttl_secs: u64,
    resumer: Arc<R>,
    /// `ref_id`s already sent, so a `pending_confirm` row isn't re-messaged
    /// every poll cycle while it waits on a reply.
    sent: Mutex<HashSet<String>>,
    login_resumer: Mutex<Option<Arc<dyn LoginResumer>>>,
    /// `true` once a login prompt has been sent and we're waiting on a
    /// reply — avoids re-sending it on every periodic re-check.
    login_awaiting: Mutex<bool>,
}

fn ist_offset() -> FixedOffset {
    FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("valid offset")
}

/// `mount_common`-style pure helpers, kept free functions so they're testable
/// without spinning up a `Confirmer`.
pub fn format_order_summary(p: &PendingConfirm) -> String {
    format!(
        "Confirm order?\n{} {} x{} @ Rs.{:.2}\nref: {}",
        p.side, p.tradingsymbol, p.quantity, p.limit_price, p.ref_id
    )
}

pub fn generate_nonce() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Parses `callback_data` of the form `"<nonce>:approve"` / `"<nonce>:reject"`
/// and checks it against the nonce on record. `None` if the callback doesn't
/// even parse (wrong shape); `Some(false)` if it parses but the nonce doesn't
/// match — both cases must be treated as "not a valid confirmation" by the
/// caller, never silently accepted.
pub fn validate_callback(data: &str, expected_nonce: &str) -> Option<bool> {
    let (nonce, action) = data.split_once(':')?;
    if !matches!(action, "approve" | "reject") {
        return None;
    }
    Some(nonce == expected_nonce)
}

pub fn is_expired(requested_at: &str, ttl_secs: u64, now: DateTime<FixedOffset>) -> bool {
    match DateTime::parse_from_rfc3339(requested_at) {
        Ok(ts) => (now - ts).num_seconds() >= ttl_secs as i64,
        // An unparsable timestamp can never be proven fresh — fail closed.
        Err(_) => true,
    }
}

impl<R: ConfirmResumer> Confirmer<R> {
    pub fn new(
        bot_token: &str,
        chat_id: String,
        ttl_secs: u64,
        ledger: Arc<Ledger>,
        resumer: Arc<R>,
    ) -> Self {
        Self {
            client: TelegramClient::new(bot_token),
            ledger,
            chat_id,
            ttl_secs,
            resumer,
            sent: Mutex::new(HashSet::new()),
            login_resumer: Mutex::new(None),
            login_awaiting: Mutex::new(false),
        }
    }

    /// Points the Telegram client at a different API root. Test-only knob
    /// (mocking the real `api.telegram.org`), not `cfg(test)`-gated because
    /// integration tests in `tests/` need it too.
    pub fn with_base_url(
        bot_token: &str,
        base_url: &str,
        chat_id: String,
        ttl_secs: u64,
        ledger: Arc<Ledger>,
        resumer: Arc<R>,
    ) -> Self {
        Self {
            client: TelegramClient::with_base_url(bot_token, base_url),
            ledger,
            chat_id,
            ttl_secs,
            resumer,
            sent: Mutex::new(HashSet::new()),
            login_resumer: Mutex::new(None),
            login_awaiting: Mutex::new(false),
        }
    }

    /// Registers the automatic-relogin handler. `None` (the default) means
    /// login prompts/replies are never sent or processed by this bot.
    pub fn with_login_resumer(self, resumer: Arc<dyn LoginResumer>) -> Self {
        *self.login_resumer.lock().unwrap() = Some(resumer);
        self
    }

    /// Call whenever the Kite token status is observed to be `NeedsLogin`. A
    /// no-op if a prompt is already outstanding or no login resumer is
    /// registered, so a periodic checker can call this freely.
    pub async fn notify_needs_login(&self, login_url: &str) -> Result<(), ConfirmError> {
        if self.login_resumer.lock().unwrap().is_none() {
            return Ok(());
        }
        {
            let mut awaiting = self.login_awaiting.lock().unwrap();
            if *awaiting {
                return Ok(());
            }
            *awaiting = true;
        }
        self.client
            .send_message(
                &self.chat_id,
                &format!(
                    "Kite session expired. Log in, then reply here with the request_token \
                     (or paste the whole redirect URL):\n\n{login_url}"
                ),
            )
            .await
    }

    async fn handle_login_message(&self, msg: &Message) {
        if msg.chat.id.to_string() != self.chat_id {
            return;
        }
        let resumer = self.login_resumer.lock().unwrap().clone();
        let Some(resumer) = resumer else { return };
        if !*self.login_awaiting.lock().unwrap() {
            return;
        }
        let Some(text) = &msg.text else { return };
        let token = extract_request_token(text);
        if token.is_empty() {
            return;
        }
        match resumer.complete_login(&token).await {
            Ok(()) => {
                *self.login_awaiting.lock().unwrap() = false;
                self.client
                    .send_message(&self.chat_id, "Logged in.")
                    .await
                    .ok();
            }
            Err(e) => {
                // Leave `awaiting` set so the user can just reply again.
                self.client
                    .send_message(
                        &self.chat_id,
                        &format!("Login failed: {e}. Reply again with a fresh request_token."),
                    )
                    .await
                    .ok();
            }
        }
    }

    async fn handle_callback(&self, cb: &CallbackQuery, pending: &[PendingConfirm]) {
        let Some(data) = cb.data.as_deref() else {
            return;
        };
        if cb.from.id.to_string() != self.chat_id {
            // Not the allowlisted chat — never resolve on this callback.
            self.client.answer_callback_query(&cb.id).await.ok();
            return;
        }
        let Some((nonce, action)) = data.split_once(':') else {
            self.client.answer_callback_query(&cb.id).await.ok();
            return;
        };
        let matching = pending.iter().find(|p| p.nonce.as_deref() == Some(nonce));
        if let Some(p) = matching {
            if validate_callback(data, nonce) == Some(true) {
                let outcome = if action == "approve" {
                    ConfirmOutcome::Approved
                } else {
                    ConfirmOutcome::Rejected
                };
                let ledger_outcome = if action == "approve" {
                    "APPROVED"
                } else {
                    "REJECTED"
                };
                self.ledger
                    .resolve_confirmation(&p.ref_id, ledger_outcome)
                    .ok();
                self.resumer.resume(&p.ref_id, outcome).await;
            }
        }
        self.client.answer_callback_query(&cb.id).await.ok();
    }

    /// DESIGN.md §2.7: must succeed before the server starts in any mode that
    /// can reach `NeedsHumanConfirm`.
    pub async fn health_check(&self) -> Result<(), ConfirmError> {
        self.client.get_me().await
    }

    /// One poll cycle: send any not-yet-sent pending confirmations, drain
    /// Telegram updates, resolve on a matching callback, and time out
    /// anything past its TTL. Returns the update offset to pass next time.
    pub async fn poll_once(&self, offset: i64) -> Result<i64, ConfirmError> {
        // Send new pending confirmations.
        let pending = self.ledger.list_pending_confirms().map_err(ledger_err)?;
        for p in &pending {
            let already_sent = self.sent.lock().unwrap().contains(&p.ref_id);
            if already_sent || p.nonce.is_some() {
                continue;
            }
            let nonce = generate_nonce();
            self.client
                .send_message_with_buttons(&self.chat_id, &format_order_summary(p), &nonce)
                .await?;
            self.ledger
                .set_confirmation_nonce(&p.ref_id, &nonce)
                .map_err(ledger_err)?;
            self.sent.lock().unwrap().insert(p.ref_id.clone());
        }

        // Time out anything past its TTL — fail closed.
        let now = Utc::now().with_timezone(&ist_offset());
        for p in &pending {
            if is_expired(&p.requested_at, self.ttl_secs, now) {
                self.ledger.resolve_confirmation(&p.ref_id, "TIMEOUT").ok();
                self.resumer
                    .resume(&p.ref_id, ConfirmOutcome::Timeout)
                    .await;
            }
        }

        // Drain updates: callback-query buttons (order confirm) and plain
        // text messages (login-token replies) share this one poll loop —
        // see the module docs for why they can't be two separate pollers.
        let updates = self.client.get_updates(offset, 1).await?;
        let mut next_offset = offset;
        for u in updates {
            next_offset = next_offset.max(u.update_id + 1);
            if let Some(cb) = &u.callback_query {
                self.handle_callback(cb, &pending).await;
            }
            if let Some(msg) = &u.message {
                self.handle_login_message(msg).await;
            }
        }

        Ok(next_offset)
    }

    /// Runs forever. Intended as a background task alongside the MCP server.
    pub async fn run(&self) {
        let mut offset = 0i64;
        loop {
            match self.poll_once(offset).await {
                Ok(next) => offset = next,
                Err(_) => {
                    // Network/Telegram failure: nothing to do but retry: the
                    // TTL sweep inside `poll_once` is what actually fails
                    // pending orders closed while the bot is unreachable.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }
}

fn ledger_err(e: ledger::LedgerError) -> ConfirmError {
    ConfirmError::Ledger(e.to_string())
}
