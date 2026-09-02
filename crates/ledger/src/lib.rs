//! Append-only journal of what the bot did (DESIGN.md §2.4, §6).
//!
//! Stub — implemented in `feat/ledger`. SQLite (WAL). Local source of truth for
//! the bot's own actions; NOT the source of truth for positions or order state
//! (those are reconciled live). Daily counters are derived by query, never
//! stored as mutable totals.

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("ledger is not implemented yet (feat/ledger)")]
    NotImplemented,
}
