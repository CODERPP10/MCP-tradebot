//! `tradebot login` — exchange a pasted Kite `request_token` for an access
//! token and seal it (DESIGN.md §2.3 / §2.9). Run once each trading morning.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use kite_client::{KiteClient, KiteConfig};
use secret_store::{shared, SealedFileStore, SecretStore, KITE_API_KEY};
use session::ManualPaste;

use crate::home;
use crate::prompt::{read_hidden, read_line};

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
    let provider = ManualPaste::new(shared(store), kite, api_key);

    println!(
        "\nOpen this URL, log in, then copy the `request_token` value from the\n\
         redirect URL (…?request_token=XXXX&action=login&status=success):\n\n  {}\n",
        provider.login_url()
    );
    let request_token = read_line("request_token")?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting async runtime")?;
    let sealed = rt
        .block_on(provider.complete_login(&request_token))
        .context("exchanging the request token with Kite")?;

    println!("\nAccess token sealed into {}.", store_path.display());
    println!("  issued_at              {}", sealed.issued_at.to_rfc3339());
    println!(
        "  assumed_invalid_after  {}   (Kite invalidates tokens ~06:00 IST)",
        sealed.assumed_invalid_after.to_rfc3339()
    );
    Ok(())
}
