//! Wiring the engine from local operator state. The MCP binary is started by
//! Claude Desktop, which cannot answer a passphrase prompt, so the store
//! passphrase comes from `TRADEBOT_PASSPHRASE`.
//!
//! **This env var is a paper-demo shortcut only** (DESIGN.md §10). Live mode
//! must move the passphrase into an OS credential store.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use domain::ConfirmOutcome;
use engine::Engine;
use guardrails::Policy;
use kite_client::{KiteClient, KiteConfig};
use ledger::Ledger;
use secret_store::{
    shared, SealedFileStore, SecretStore, KITE_API_KEY, TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID,
};
use session::{ManualPaste, TokenProvider, TokenStatus};
use telegram_confirm::{ConfirmResumer, Confirmer, LoginResumer};

pub const PASSPHRASE_ENV: &str = "TRADEBOT_PASSPHRASE";
pub const HOME_ENV: &str = "TRADEBOT_HOME";
pub const POLICY_PATH_ENV: &str = "TRADEBOT_POLICY_PATH";

/// Bridges `telegram-confirm` back to `engine::resume_confirmed` without
/// either crate depending on the other (see `domain::ConfirmOutcome`).
#[derive(Clone)]
pub struct EngineResumer(pub Engine);

#[async_trait]
impl ConfirmResumer for EngineResumer {
    async fn resume(&self, ref_id: &str, outcome: ConfirmOutcome) {
        if let Err(e) = self.0.resume_confirmed(ref_id, outcome).await {
            eprintln!("tradebot-mcp: resume_confirmed({ref_id}) failed: {e}");
        }
    }
}

/// Bridges `telegram-confirm`'s automatic-relogin flow back to
/// `session::ManualPaste::complete_login`, the same decoupling pattern as
/// [`EngineResumer`].
struct KiteLoginResumer(ManualPaste);

#[async_trait]
impl LoginResumer for KiteLoginResumer {
    async fn complete_login(&self, request_token: &str) -> Result<(), String> {
        self.0
            .complete_login(request_token)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// How often to check whether the Kite session has expired and, if so, nudge
/// Telegram. Kite invalidates tokens once a day (~06:00 IST); this just needs
/// to notice within a few minutes of that, not immediately.
const LOGIN_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// How often the background reconciliation pass runs (DESIGN.md §4). It polls
/// Kite's order book to drive open orders to a terminal state and to flag
/// divergences; it never places or re-sends an order, so a slow cadence only
/// delays *noticing*, it cannot cause a trade. One minute keeps the Kite rate
/// budget free for the order path, which reconciles its own context live
/// before every decision anyway.
const RECONCILE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

pub struct Boot {
    pub engine: Engine,
    /// `None` when no Telegram credentials are sealed — orders that need
    /// confirmation still get a `pending_confirm` ledger row, but only the
    /// `tradebot confirm`/`reject` CLI fallback can resolve them. Automatic
    /// Telegram relogin is also unavailable in that case — use `tradebot
    /// login` instead.
    pub confirmer: Option<Arc<Confirmer<EngineResumer>>>,
}

pub async fn build_engine() -> Result<Boot> {
    let home = std::env::var_os(HOME_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".tradebot"));
    let store_path = home.join("secrets.json");
    if !store_path.exists() {
        bail!(
            "no sealed store at {} — run `tradebot init` then `tradebot login` first",
            store_path.display()
        );
    }

    let passphrase = std::env::var(PASSPHRASE_ENV).with_context(|| {
        format!("{PASSPHRASE_ENV} must be set — Claude Desktop cannot prompt (paper-demo only)")
    })?;

    let store = SealedFileStore::open(&store_path, &passphrase)
        .context("opening the sealed store (wrong passphrase?)")?;
    let api_key = store
        .get(KITE_API_KEY)
        .context("reading the Kite API key")?;
    let telegram_creds = store
        .get(TELEGRAM_BOT_TOKEN)
        .ok()
        .zip(store.get(TELEGRAM_CHAT_ID).ok());

    let kite = KiteClient::new(KiteConfig::new(api_key.clone())).context("building Kite client")?;
    let manual_paste = ManualPaste::new(shared(store), kite.clone(), api_key);
    let tokens: Arc<dyn TokenProvider> = Arc::new(manual_paste.clone());

    let policy_path = std::env::var_os(POLICY_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("policy.toml"));
    let policy = Arc::new(
        Policy::load(&policy_path)
            .with_context(|| format!("loading guardrail policy from {}", policy_path.display()))?,
    );

    let ledger =
        Arc::new(Ledger::open(home.join("ledger.db")).context("opening the ledger database")?);

    let engine = Engine::with_submit(kite, tokens.clone(), ledger.clone(), policy.clone());

    let confirmer = match telegram_creds {
        Some((bot_token, chat_id)) => {
            let resumer = Arc::new(EngineResumer(engine.clone()));
            let confirmer = Confirmer::new(
                &bot_token,
                chat_id,
                policy.confirmation.confirm_ttl_secs,
                ledger,
                resumer,
            )
            .with_login_resumer(Arc::new(KiteLoginResumer(manual_paste.clone())));
            confirmer
                .health_check()
                .await
                .context("Telegram getMe health check failed — refusing to start with a broken confirm channel")?;
            let confirmer = Arc::new(confirmer);

            let watcher_tokens = tokens.clone();
            let watcher_confirmer = confirmer.clone();
            tokio::spawn(async move {
                loop {
                    if let Ok(TokenStatus::NeedsLogin(n)) = watcher_tokens.access_token().await {
                        if let Err(e) = watcher_confirmer.notify_needs_login(&n.login_url).await {
                            eprintln!("tradebot-mcp: notify_needs_login failed: {e}");
                        }
                    }
                    tokio::time::sleep(LOGIN_CHECK_INTERVAL).await;
                }
            });

            Some(confirmer)
        }
        None => {
            eprintln!(
                "tradebot-mcp: no Telegram credentials sealed — orders needing confirmation \
                 will only be resolvable via `tradebot confirm`/`tradebot reject`"
            );
            None
        }
    };

    // DESIGN.md §4: reconcile on startup and then continuously. The loop runs
    // its first pass immediately, which is what covers "on startup"; the
    // "before every order decision" half is already handled inside
    // `engine::submit`, which pulls its `AccountContext` live.
    let reconciler = engine.clone();
    tokio::spawn(async move { reconciler.reconcile_loop(RECONCILE_INTERVAL).await });

    Ok(Boot { engine, confirmer })
}
