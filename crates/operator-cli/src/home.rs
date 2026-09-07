//! Resolving `TRADEBOT_HOME` — the directory holding the sealed secret store,
//! the ledger DB, and the instrument cache. Shared by every subcommand.

use std::path::PathBuf;

/// The tradebot data directory: `--home`, else `$TRADEBOT_HOME`, else
/// `./.tradebot`.
pub fn resolve(explicit: Option<PathBuf>) -> PathBuf {
    explicit
        .or_else(|| std::env::var_os("TRADEBOT_HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(".tradebot"))
}

/// Path to the sealed secret store within a resolved home dir.
pub fn secret_store_path(home: &std::path::Path) -> PathBuf {
    home.join("secrets.json")
}
