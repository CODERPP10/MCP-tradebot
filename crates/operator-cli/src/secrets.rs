//! `tradebot secrets put <key>` / `tradebot secrets list` — seal an
//! additional record (e.g. Telegram credentials) into an *existing* store
//! without the destructive re-`init` that would lose the Kite key/secret and
//! any sealed access token.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use secret_store::{SealedFileStore, SecretStore};

use crate::home;
use crate::prompt::read_hidden;

pub fn put(explicit_home: Option<PathBuf>, key: &str) -> Result<()> {
    let home = home::resolve(explicit_home);
    let store_path = home::secret_store_path(&home);
    if !store_path.exists() {
        bail!(
            "no sealed store at {} — run `tradebot init` first",
            store_path.display()
        );
    }
    let passphrase = read_hidden("Store passphrase")?;
    let mut store = SealedFileStore::open(&store_path, &passphrase)
        .context("opening the sealed store (wrong passphrase?)")?;
    let value = read_hidden(&format!("Value for {key}"))?;
    if value.trim().is_empty() {
        bail!("value must not be empty");
    }
    store.put(key, value.trim()).context("sealing the record")?;
    println!("sealed {key}");
    Ok(())
}

pub fn list(explicit_home: Option<PathBuf>) -> Result<()> {
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
    for key in store.list_keys() {
        println!("{key}");
    }
    Ok(())
}
