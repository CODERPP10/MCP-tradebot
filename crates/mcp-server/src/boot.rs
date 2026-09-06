//! Wiring the engine from local operator state. The MCP binary is started by
//! Claude Desktop, which cannot answer a passphrase prompt, so the store
//! passphrase comes from `TRADEBOT_PASSPHRASE`.
//!
//! **This env var is a paper-demo shortcut only** (DESIGN.md §10). Live mode
//! must move the passphrase into an OS credential store.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use engine::Engine;
use kite_client::{KiteClient, KiteConfig};
use secret_store::{shared, SealedFileStore, SecretStore, KITE_API_KEY};
use session::{ManualPaste, TokenProvider};

pub const PASSPHRASE_ENV: &str = "TRADEBOT_PASSPHRASE";
pub const HOME_ENV: &str = "TRADEBOT_HOME";

pub fn build_engine() -> Result<Engine> {
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

    let kite = KiteClient::new(KiteConfig::new(api_key.clone())).context("building Kite client")?;
    let tokens: Arc<dyn TokenProvider> =
        Arc::new(ManualPaste::new(shared(store), kite.clone(), api_key));

    Ok(Engine::new(kite, tokens))
}
