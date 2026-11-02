//! `tradebot reconcile` — force one reconciliation pass and print what it
//! found (DESIGN.md §2.9, §4).
//!
//! The same `Engine::reconcile_once` the `tradebot-mcp` background loop runs,
//! driven by hand. It reads Kite's order book and *writes only to the ledger*
//! (status rows, `submit_failed` marks, divergences) — it never places,
//! modifies, cancels or re-sends an order.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use engine::Engine;
use guardrails::Policy;
use kite_client::{KiteClient, KiteConfig};
use ledger::Ledger;
use secret_store::{shared, SealedFileStore, SecretStore, KITE_API_KEY};
use session::ManualPaste;

use crate::home;
use crate::prompt::read_hidden;

/// How many standing divergences to list after the pass. Divergences are
/// deduplicated per observed state, so this is a short history, not a count
/// of polls.
const DIVERGENCE_HISTORY: u32 = 20;

pub fn run(explicit_home: Option<PathBuf>) -> Result<()> {
    let home = home::resolve(explicit_home);
    let store_path = home::secret_store_path(&home);
    if !store_path.exists() {
        bail!(
            "no sealed store at {} — run `tradebot init` first",
            store_path.display()
        );
    }
    let passphrase = read_hidden("Store passphrase")?;
    let store = SealedFileStore::open(&store_path, &passphrase)
        .context("opening the sealed store (wrong passphrase?)")?;
    let api_key = store
        .get(KITE_API_KEY)
        .context("reading the Kite API key")?;
    let kite = KiteClient::new(KiteConfig::new(api_key.clone())).context("building Kite client")?;
    let tokens = Arc::new(ManualPaste::new(shared(store), kite.clone(), api_key));

    let policy = Arc::new(Policy::load("policy.toml").context("loading policy.toml")?);
    let ledger = Arc::new(Ledger::open(home::ledger_db_path(&home)).context("opening the ledger")?);
    let engine = Engine::with_submit(kite, tokens, ledger.clone(), policy);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting async runtime")?;

    let report = rt
        .block_on(engine.reconcile_once())
        .context("reconciliation pass failed")?;

    println!(
        "broker orders examined     {}",
        report.broker_orders_examined
    );
    println!(
        "open submissions examined  {}",
        report.ledger_submissions_examined
    );
    println!("status rows appended       {}", report.statuses_updated);
    println!(
        "marked submit_failed       {}",
        report.submissions_marked_failed
    );
    println!("re-linked by tag           {}", report.relinked_by_tag);

    println!("\nNew divergences this pass ({})", report.divergences.len());
    for d in &report.divergences {
        println!("  {} [{}] {}", d.observed_at, d.kind, d.detail_json);
    }
    if report.divergences.is_empty() {
        println!("  (none — identical repeats of a known divergence are not re-recorded)");
    }

    let standing = ledger
        .list_divergences(DIVERGENCE_HISTORY)
        .context("reading divergence history")?;
    println!("\nMost recent divergences on record ({})", standing.len());
    for d in &standing {
        println!("  {} [{}] {}", d.observed_at, d.kind, d.detail_json);
    }
    if standing.is_empty() {
        println!("  (none)");
    }

    if report.submissions_marked_failed > 0 {
        println!(
            "\nNote: submissions marked `submit_failed` are never auto-resent. Check the \
             broker's order book before re-placing anything by hand."
        );
    }
    Ok(())
}
