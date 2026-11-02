//! Manual, live smoke test for the Telegram confirm flow (paper mode only).
//!
//! Builds the same `Engine` + `Confirmer` wiring as `tradebot-mcp serve`, but
//! also submits one tiny order itself and prints what happens to it —
//! including the confirm outcome, which the CLI has no other way to surface
//! yet. Never touches a live Kite mutating endpoint (`PaperExecutor` only).
//!
//! Run with the same env/cwd conventions as `tradebot-mcp`:
//!   TRADEBOT_PASSPHRASE=... cargo run -p mcp-server --example smoke_confirm -- INFY
//!
//! Requires: `tradebot init` + `tradebot login` already done, and Telegram
//! credentials sealed via `tradebot secrets put telegram_bot_token` /
//! `telegram_chat_id`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use domain::{
    ConfirmOutcome, Exchange, OrderIntent, OrderType, Product, Quantity, Validity, Variety,
};
use engine::{Engine, IntentSource, SubmitOutcome};
use guardrails::Policy;
use kite_client::{KiteClient, KiteConfig};
use ledger::Ledger;
use secret_store::{
    shared, SealedFileStore, SecretStore, KITE_API_KEY, TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID,
};
use session::{ManualPaste, TokenProvider, TokenStatus};
use telegram_confirm::{ConfirmResumer, Confirmer};

struct PrintingResumer(Engine);

#[async_trait]
impl ConfirmResumer for PrintingResumer {
    async fn resume(&self, ref_id: &str, outcome: ConfirmOutcome) {
        println!("\n>>> Telegram resolved {ref_id} as {outcome:?}, resuming...");
        match self.0.resume_confirmed(ref_id, outcome).await {
            Ok(SubmitOutcome::Accepted { order_id, .. }) => {
                println!(">>> paper-filled as {order_id}");
            }
            Ok(SubmitOutcome::Rejected { reasons, .. }) => {
                println!(">>> rejected: {reasons:?}");
            }
            Ok(SubmitOutcome::Pending { .. }) => println!(">>> unexpected: still pending"),
            Err(e) => println!(">>> resume_confirmed failed: {e}"),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let tradingsymbol = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "INFY".to_string());

    let home = std::env::var_os("TRADEBOT_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".tradebot"));
    let passphrase = std::env::var("TRADEBOT_PASSPHRASE")
        .context("set TRADEBOT_PASSPHRASE (the sealed-store passphrase)")?;
    let store = SealedFileStore::open(home.join("secrets.json"), &passphrase)
        .context("opening the sealed store (wrong passphrase?)")?;
    let api_key = store.get(KITE_API_KEY).context("reading Kite API key")?;
    let bot_token = store
        .get(TELEGRAM_BOT_TOKEN)
        .context("no telegram_bot_token sealed — run `tradebot secrets put telegram_bot_token`")?;
    let chat_id = store
        .get(TELEGRAM_CHAT_ID)
        .context("no telegram_chat_id sealed — run `tradebot secrets put telegram_chat_id`")?;

    let kite = KiteClient::new(KiteConfig::new(api_key.clone()))?;
    let tokens: Arc<dyn TokenProvider> =
        Arc::new(ManualPaste::new(shared(store), kite.clone(), api_key));
    match tokens.access_token().await.context("checking session")? {
        TokenStatus::NeedsLogin(n) => bail!("run `tradebot login` first — {}", n.login_url),
        TokenStatus::Valid(t) => kite.set_access_token(t).await,
    }

    let policy = Arc::new(Policy::load("policy.toml").context("loading policy.toml")?);
    let ledger = Arc::new(Ledger::open(home.join("ledger.db")).context("opening ledger")?);
    let engine = Engine::with_submit(kite, tokens, ledger.clone(), policy.clone());

    let resumer = Arc::new(PrintingResumer(engine.clone()));
    let confirmer = Arc::new(Confirmer::new(
        &bot_token,
        chat_id,
        policy.confirmation.confirm_ttl_secs,
        ledger,
        resumer,
    ));
    confirmer
        .health_check()
        .await
        .context("Telegram getMe failed")?;
    println!("Telegram bot reachable. Starting confirm poller...");
    let poller = {
        let confirmer = confirmer.clone();
        tokio::spawn(async move { confirmer.run().await })
    };

    // `engine.quote()` is a plain read pass-through with no fallback — unlike
    // `engine::submit`'s internal `account_context()`, which already falls
    // back to positions/holdings `last_price` on a `PermissionException`. Do
    // the same here just to pick a demo limit price, so this example works
    // on an account without the market-data add-on too.
    let limit_price = match engine.quote(&tradingsymbol).await {
        Ok(q) => domain::Money::from_rupees_f64(q.last_price)?,
        Err(_) => {
            println!(
                "live quote unavailable (no market-data add-on?) — falling back to a held \
                 position/holding's last_price..."
            );
            let symbol = tradingsymbol.trim().to_uppercase();
            let from_positions = engine.positions().await.ok().and_then(|p| {
                p.net
                    .iter()
                    .find(|r| r.tradingsymbol.trim().to_uppercase() == symbol)
                    .map(|r| r.last_price)
            });
            let price = match from_positions {
                Some(p) => Some(p),
                None => engine.holdings().await.ok().and_then(|h| {
                    h.holdings
                        .iter()
                        .find(|r| r.tradingsymbol.trim().to_uppercase() == symbol)
                        .map(|r| r.last_price)
                }),
            }
            .with_context(|| {
                format!(
                    "no live quote and {tradingsymbol} isn't held anywhere — pass a symbol you \
                     currently hold a position or holding in, or buy the Kite market-data add-on"
                )
            })?;
            domain::Money::from_rupees_f64(price)?
        }
    };
    println!(
        "{tradingsymbol} reference price is {limit_price}; submitting a 1-share BUY at that \
         price..."
    );

    let intent = OrderIntent {
        tradingsymbol,
        exchange: Exchange::Nse,
        side: domain::Side::Buy,
        quantity: Quantity::new(1)?,
        product: Product::Cnc,
        variety: Variety::Regular,
        order_type: OrderType::Limit,
        validity: Validity::Day,
        limit_price: Some(limit_price),
    };

    match engine.submit(intent, IntentSource::Cli).await? {
        SubmitOutcome::Pending { ref_id } => {
            println!("pending_confirmation: {ref_id} — check Telegram and tap Approve/Reject.");
            println!(
                "Waiting up to {}s...",
                policy.confirmation.confirm_ttl_secs + 15
            );
            tokio::time::sleep(Duration::from_secs(
                policy.confirmation.confirm_ttl_secs + 15,
            ))
            .await;
        }
        SubmitOutcome::Accepted { order_id, ref_id } => {
            println!("accepted immediately (no confirm needed): {order_id} / {ref_id}");
        }
        SubmitOutcome::Rejected { reasons, .. } => {
            println!("rejected: {reasons:?}");
        }
    }

    poller.abort();
    Ok(())
}
