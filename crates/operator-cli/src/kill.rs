//! `tradebot kill` / `tradebot resume` — toggle the ledger kill switch
//! (DESIGN.md §2.9). Audited: every toggle writes an `operator_audit` row.

use std::path::PathBuf;

use anyhow::{Context, Result};
use ledger::Ledger;

use crate::home;

fn set(explicit_home: Option<PathBuf>, engaged: bool) -> Result<()> {
    let home = home::resolve(explicit_home);
    let ledger = Ledger::open(home::ledger_db_path(&home)).context("opening the ledger")?;
    ledger
        .set_kill_switch(engaged, "operator", None)
        .context("writing the kill switch")?;
    println!(
        "kill switch {}",
        if engaged {
            "ENGAGED — all mutating orders will be rejected"
        } else {
            "disengaged"
        }
    );
    Ok(())
}

pub fn kill(explicit_home: Option<PathBuf>) -> Result<()> {
    set(explicit_home, true)
}

pub fn resume(explicit_home: Option<PathBuf>) -> Result<()> {
    set(explicit_home, false)
}
