//! `tradebot confirm <ref_id>` / `tradebot reject <ref_id>` — the CLI
//! fallback for approving a pending order without Telegram (DESIGN.md §2.7,
//! §2.9). Resolves the same ledger row and takes the same `engine` path as an
//! approved Telegram callback.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use domain::ConfirmOutcome;
use engine::{Engine, SubmitOutcome};
use guardrails::Policy;
use kite_client::{KiteClient, KiteConfig};
use ledger::Ledger;
use secret_store::{shared, SealedFileStore, SecretStore, KITE_API_KEY};
use session::ManualPaste;

use crate::home;
use crate::prompt::read_hidden;

fn resolve(explicit_home: Option<PathBuf>, ref_id: &str, outcome: ConfirmOutcome) -> Result<()> {
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
    let engine = Engine::with_submit(kite, tokens, ledger, policy);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting async runtime")?;

    rt.block_on(async {
        match engine.resume_confirmed(ref_id, outcome).await {
            Ok(SubmitOutcome::Accepted { order_id, .. }) => {
                println!("approved — paper-filled as {order_id}");
            }
            Ok(SubmitOutcome::Rejected { reasons, .. }) => {
                println!("resolved as rejected: {reasons:?}");
            }
            Ok(SubmitOutcome::Pending { .. }) => {
                println!("unexpected: still pending after resolution");
            }
            Err(e) => bail!("resolving {ref_id} failed: {e}"),
        }
        Ok(())
    })
}

pub fn confirm(explicit_home: Option<PathBuf>, ref_id: &str) -> Result<()> {
    resolve(explicit_home, ref_id, ConfirmOutcome::Approved)
}

pub fn reject(explicit_home: Option<PathBuf>, ref_id: &str) -> Result<()> {
    resolve(explicit_home, ref_id, ConfirmOutcome::Rejected)
}
