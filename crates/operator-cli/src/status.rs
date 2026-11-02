//! `tradebot status` — the operator's one-screen answer to "is this thing
//! safe and what has it done today?" (DESIGN.md §2.9): token validity,
//! kill-switch state, today's ledger aggregates, pending confirmations and
//! the last reconciliation divergence.
//!
//! Every ledger read here is local. The session check needs the store
//! passphrase, so `--no-session` skips it (and is also what to use when the
//! store does not exist yet).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ledger::Ledger;
use secret_store::{shared, SealedFileStore, SecretStore, KITE_API_KEY};
use session::{ManualPaste, TokenProvider, TokenStatus};

use crate::home;
use crate::prompt::read_hidden;

pub fn run(explicit_home: Option<PathBuf>, no_session: bool) -> Result<()> {
    let home = home::resolve(explicit_home);
    println!("home  {}", home.display());

    println!("\nSession");
    if no_session {
        println!("  skipped (--no-session)");
    } else {
        match session_summary(&home) {
            Ok(lines) => {
                for line in lines {
                    println!("  {line}");
                }
            }
            // A session the operator can't read is worth reporting, not worth
            // aborting the rest of the status for.
            Err(err) => println!("  unavailable: {err:#}"),
        }
    }

    let db = home::ledger_db_path(&home);
    if !db.exists() {
        println!(
            "\nLedger\n  no ledger at {} yet — nothing has run",
            db.display()
        );
        return Ok(());
    }
    let ledger = Ledger::open(&db).context("opening the ledger")?;

    let kill = ledger.kill_switch().context("reading the kill switch")?;
    println!("\nKill switch");
    if kill.engaged {
        println!("  ENGAGED — all mutating orders are rejected");
    } else {
        println!("  disengaged");
    }
    if let Some(at) = &kill.changed_at {
        let by = kill.changed_by.as_deref().unwrap_or("unknown");
        println!("  last changed {at} by {by}");
    }
    if let Some(note) = &kill.note {
        println!("  note: {note}");
    }

    let agg = ledger
        .daily_aggregates()
        .context("reading today's aggregates")?;
    println!("\nToday (IST)");
    println!("  orders submitted   {}", agg.orders_today);
    println!("  gross notional     {:.2}", agg.gross_notional_today);
    println!("  net new capital    {:.2}", agg.net_new_capital_today);
    match agg.last_order_ts {
        Some(ts) => println!("  last order         {}", ts.to_rfc3339()),
        None => println!("  last order         (never)"),
    }

    let pending = ledger
        .list_pending_confirms()
        .context("reading pending confirmations")?;
    println!("\nPending confirmations ({})", pending.len());
    for p in &pending {
        println!(
            "  {}  {} {} {} @ {:.2}  requested {}",
            p.ref_id, p.side, p.quantity, p.tradingsymbol, p.limit_price, p.requested_at
        );
    }
    if !pending.is_empty() {
        println!("  resolve with: tradebot confirm <ref_id> | tradebot reject <ref_id>");
    }

    println!("\nReconciliation");
    println!("  background loop not implemented yet (roadmap item 8)");
    match ledger.last_divergence().context("reading divergences")? {
        Some(d) => println!(
            "  last divergence {} [{}] {}",
            d.observed_at, d.kind, d.detail_json
        ),
        None => println!("  no divergences recorded"),
    }

    Ok(())
}

/// Token validity, read entirely from the sealed store — no Kite call, so
/// this says what the bot *believes* about its token, not what Kite would
/// answer. `tradebot whoami` is the live check.
fn session_summary(home: &Path) -> Result<Vec<String>> {
    let store_path = home::secret_store_path(home);
    if !store_path.exists() {
        return Ok(vec![format!(
            "no sealed store at {} — run `tradebot init`",
            store_path.display()
        )]);
    }
    let passphrase = read_hidden("Store passphrase")?;
    let store = SealedFileStore::open(&store_path, &passphrase)
        .context("opening the sealed store (wrong passphrase?)")?;
    let api_key = store
        .get(KITE_API_KEY)
        .context("reading the Kite API key")?;
    let kite = kite_client::KiteClient::new(kite_client::KiteConfig::new(api_key.clone()))
        .context("building Kite client")?;
    let provider = ManualPaste::new(shared(store), kite, api_key);

    let sealed = provider.sealed_token().context("reading sealed token")?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting async runtime")?;
    let status = rt
        .block_on(provider.access_token())
        .context("evaluating token status")?;

    let mut lines = Vec::new();
    match status {
        TokenStatus::Valid(_) => lines.push("token believed valid".to_string()),
        TokenStatus::NeedsLogin(n) => {
            lines.push("token expired or absent — run `tradebot login`".to_string());
            lines.push(format!("login URL: {}", n.login_url));
        }
    }
    if let Some(t) = sealed {
        lines.push(format!("issued          {}", t.issued_at.to_rfc3339()));
        lines.push(format!(
            "assumed invalid {}",
            t.assumed_invalid_after.to_rfc3339()
        ));
    }
    lines.push("live check: tradebot whoami".to_string());
    Ok(lines)
}
