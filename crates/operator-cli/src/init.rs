//! `tradebot init` — create the sealed store and seal the operator's
//! credentials into it (DESIGN.md §2.9).

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use secret_store::{
    Argon2Params, SealedFileStore, SecretStore, KITE_API_KEY, KITE_API_SECRET, TELEGRAM_BOT_TOKEN,
    TELEGRAM_CHAT_ID,
};

use crate::home;

const MIN_PASSPHRASE_LEN: usize = 8;

pub fn run(explicit_home: Option<PathBuf>) -> Result<()> {
    let home = home::resolve(explicit_home);
    let store_path = home::secret_store_path(&home);

    if store_path.exists() {
        bail!(
            "{} already exists — refusing to overwrite an existing store.\n\
             Delete it by hand if you really want to start over.",
            store_path.display()
        );
    }
    std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;

    println!(
        "Creating a new sealed secret store at {}",
        store_path.display()
    );
    println!(
        "\nThe passphrase seals every secret at rest. It is required on every\n\
         start of the server and the CLI, and it CANNOT be recovered — if you\n\
         lose it, you re-run `tradebot init` and re-enter your credentials.\n"
    );

    let passphrase = read_new_passphrase()?;
    let api_key = read_line("Kite API key")?;
    let api_secret = read_secret("Kite API secret")?;
    let tg_token = read_optional_secret("Telegram bot token (optional, Enter to skip)")?;
    let tg_chat = if tg_token.is_some() {
        Some(read_line("Telegram chat id")?)
    } else {
        None
    };

    let mut store = SealedFileStore::create(&store_path, &passphrase, Argon2Params::default())
        .context("creating sealed store")?;
    store.put(KITE_API_KEY, api_key.trim())?;
    store.put(KITE_API_SECRET, api_secret.trim())?;
    if let Some(t) = &tg_token {
        store.put(TELEGRAM_BOT_TOKEN, t.trim())?;
    }
    if let Some(c) = &tg_chat {
        store.put(TELEGRAM_CHAT_ID, c.trim())?;
    }

    println!(
        "\nSealed store created with {} record(s): {}",
        store.list_keys().len(),
        store.list_keys().join(", ")
    );
    println!("Next: `tradebot login` each trading morning to seal a fresh access token.");
    Ok(())
}

/// Read one hidden value. On an interactive terminal the input is not echoed
/// (`rpassword`); when stdin is a pipe (scripts, CI) a plain line is read so
/// `init` stays automatable.
fn read_hidden(label: &str) -> Result<String> {
    if io::stdin().is_terminal() {
        Ok(rpassword::prompt_password(format!("{label}: "))?)
    } else {
        read_line_raw(label)
    }
}

fn read_line(label: &str) -> Result<String> {
    let s = read_line_raw(label)?;
    if s.is_empty() {
        bail!("{label} must not be empty");
    }
    Ok(s)
}

fn read_line_raw(label: &str) -> Result<String> {
    if io::stdin().is_terminal() {
        print!("{label}: ");
        io::stdout().flush()?;
    }
    let mut s = String::new();
    if io::stdin().read_line(&mut s).context("reading stdin")? == 0 {
        bail!("unexpected end of input while reading {label}");
    }
    Ok(s.trim().to_string())
}

fn read_new_passphrase() -> Result<String> {
    let a = read_hidden("New passphrase")?;
    if a.len() < MIN_PASSPHRASE_LEN {
        bail!("passphrase must be at least {MIN_PASSPHRASE_LEN} characters");
    }
    let b = read_hidden("Confirm passphrase")?;
    if a != b {
        bail!("passphrases did not match");
    }
    Ok(a)
}

fn read_secret(label: &str) -> Result<String> {
    let s = read_hidden(label)?;
    if s.trim().is_empty() {
        bail!("{label} must not be empty");
    }
    Ok(s)
}

fn read_optional_secret(label: &str) -> Result<Option<String>> {
    let s = read_hidden(label)?;
    Ok(if s.trim().is_empty() { None } else { Some(s) })
}
