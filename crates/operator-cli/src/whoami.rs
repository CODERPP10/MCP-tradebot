//! `tradebot whoami` — prove the sealed session works against live Kite.
//!
//! Opens the store, asks the [`session`] layer for the current access token,
//! installs it on a [`KiteClient`], and makes two **read-only** calls
//! (`/user/profile`, `/user/margins/equity`). Places no orders.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use kite_client::{KiteClient, KiteConfig};
use secret_store::{shared, SealedFileStore, SecretStore, KITE_API_KEY};
use session::{ManualPaste, TokenProvider, TokenStatus};

use crate::home;
use crate::prompt::read_hidden;

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
    let provider = ManualPaste::new(shared(store), kite.clone(), api_key);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting async runtime")?;

    rt.block_on(async {
        match provider
            .access_token()
            .await
            .context("reading sealed token")?
        {
            TokenStatus::NeedsLogin(n) => {
                bail!(
                    "no valid access token — run `tradebot login` first.\n  login URL: {}",
                    n.login_url
                );
            }
            TokenStatus::Valid(token) => kite.set_access_token(token).await,
        }

        let profile = kite
            .profile()
            .await
            .context("GET /user/profile (token rejected by Kite?)")?;
        let margins = kite
            .margins_equity()
            .await
            .context("GET /user/margins/equity")?;

        println!("Session is live.\n");
        println!("  user_id  {}", profile.user_id);
        println!("  name     {}", profile.user_name);
        println!("  broker   {}", profile.broker);
        println!("  email    {}", profile.email);
        println!("\n  equity margin");
        println!("    net              {:.2}", margins.net);
        println!("    available cash   {:.2}", margins.available.cash);
        println!("    live balance     {:.2}", margins.available.live_balance);
        println!("    used (debits)    {:.2}", margins.utilised.debits);
        Ok(())
    })
}
