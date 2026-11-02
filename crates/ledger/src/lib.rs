//! Append-only journal of what the bot did (DESIGN.md §2.4, §6).
//!
//! SQLite (WAL). Local source of truth for the bot's own actions — **not** the
//! source of truth for positions or order state (those are reconciled live by
//! `engine`). Daily counters are derived by query, never stored as mutable
//! totals, so they can never drift from the rows that produced them.

mod schema;

use std::sync::Mutex;

use chrono::{DateTime, FixedOffset, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use ulid::Ulid;

/// India Standard Time, a fixed +05:30 offset (no DST).
fn ist_offset() -> FixedOffset {
    FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("valid fixed offset")
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("ledger database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("ledger json encoding error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no such ref_id: {0}")]
    NotFound(String),
    #[error("ledger mutex poisoned")]
    Poisoned,
}

fn now_ist() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&ist_offset())
}

fn fmt_ts(ts: DateTime<FixedOffset>) -> String {
    ts.to_rfc3339()
}

/// Where an intent originated. Mirrors `intents.source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Mcp,
    Cli,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Source::Mcp => "mcp",
            Source::Cli => "cli",
        }
    }
}

/// Fields needed to write the `intents` row. Deliberately decoupled from
/// `domain::OrderIntent` so `ledger` does not have to track that type's shape
/// beyond what it persists.
#[derive(Debug, Clone)]
pub struct NewIntent {
    pub source: Source,
    pub tradingsymbol: String,
    pub exchange: String,
    pub side: String,
    pub quantity: i64,
    pub order_type: String,
    pub product: String,
    pub limit_price: f64,
}

/// `DESIGN.md §2.4`: `H(tradingsymbol, side, qty, order_type, price_bucket, epoch_window)`.
///
/// `price_bucket_pct` quantizes the price so near-identical limit prices
/// (within roughly that percentage) collapse into the same bucket;
/// `window_secs` quantizes time into non-overlapping windows, so the same hash
/// can only ever be produced within one window — a duplicate-existence check
/// on `intent_hash` alone is therefore window-scoped without a separate time
/// range query.
#[allow(clippy::too_many_arguments)]
pub fn intent_hash(
    tradingsymbol: &str,
    side: &str,
    quantity: i64,
    order_type: &str,
    limit_price: f64,
    price_bucket_pct: f64,
    window_secs: u64,
    now_unix: i64,
) -> String {
    use sha2::{Digest, Sha256};

    let bucket = if price_bucket_pct > 0.0 && limit_price > 0.0 {
        let resolution = limit_price * price_bucket_pct / 100.0;
        (limit_price / resolution).round() as i64
    } else {
        0
    };
    let window = if window_secs > 0 {
        now_unix / window_secs as i64
    } else {
        now_unix
    };

    let mut h = Sha256::new();
    h.update(tradingsymbol.trim().to_uppercase().as_bytes());
    h.update(b"|");
    h.update(side.as_bytes());
    h.update(b"|");
    h.update(quantity.to_string().as_bytes());
    h.update(b"|");
    h.update(order_type.as_bytes());
    h.update(b"|");
    h.update(bucket.to_string().as_bytes());
    h.update(b"|");
    h.update(window.to_string().as_bytes());
    hex::encode(h.finalize())
}

#[derive(Debug, Clone)]
pub struct PendingConfirm {
    pub ref_id: String,
    pub tradingsymbol: String,
    pub side: String,
    pub quantity: i64,
    pub limit_price: f64,
    pub requested_at: String,
    pub nonce: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DailyAggregates {
    pub orders_today: u32,
    pub gross_notional_today: f64,
    pub net_new_capital_today: f64,
    pub last_order_ts: Option<DateTime<FixedOffset>>,
}

pub struct Ledger {
    conn: Mutex<Connection>,
}

impl Ledger {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, LedgerError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(schema::SCHEMA_SQL)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self, LedgerError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(schema::SCHEMA_SQL)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, LedgerError> {
        self.conn.lock().map_err(|_| LedgerError::Poisoned)
    }

    // ---- intents / decisions ------------------------------------------

    /// Insert the `intents` row, returning a fresh ULID `ref_id`.
    pub fn record_intent(&self, intent: &NewIntent, hash: &str) -> Result<String, LedgerError> {
        let ref_id = Ulid::new().to_string();
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO intents (ref_id, created_at, source, tradingsymbol, exchange, side, \
             quantity, order_type, product, limit_price, intent_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                ref_id,
                fmt_ts(now_ist()),
                intent.source.as_str(),
                intent.tradingsymbol,
                intent.exchange,
                intent.side,
                intent.quantity,
                intent.order_type,
                intent.product,
                intent.limit_price,
                hash,
            ],
        )?;
        Ok(ref_id)
    }

    /// `true` if a prior intent already carries this exact hash (guardrail
    /// check #11 — `intent_hash` bakes in the idempotency window, see
    /// [`intent_hash`]).
    pub fn duplicate_within_window(&self, hash: &str) -> Result<bool, LedgerError> {
        let conn = self.lock()?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM intents WHERE intent_hash = ?1)",
            params![hash],
            |row| row.get(0),
        )?;
        Ok(exists)
    }

    pub fn record_decision(
        &self,
        ref_id: &str,
        decision: &str,
        policy_version: &str,
        reasons_json: &str,
        ltp_at_decision: Option<f64>,
    ) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO decisions (ref_id, decided_at, decision, policy_version, reasons_json, \
             ltp_at_decision) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ref_id,
                fmt_ts(now_ist()),
                decision,
                policy_version,
                reasons_json,
                ltp_at_decision,
            ],
        )?;
        Ok(())
    }

    // ---- confirmations ---------------------------------------------------

    pub fn record_confirmation_requested(
        &self,
        ref_id: &str,
        channel: &str,
        nonce: Option<&str>,
    ) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO confirmations (ref_id, channel, nonce, requested_at, resolved_at, \
             outcome) VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            params![ref_id, channel, nonce, fmt_ts(now_ist())],
        )?;
        Ok(())
    }

    /// Set the nonce on an already-`record_confirmation_requested` row, once
    /// `telegram-confirm` has actually sent the message and minted one.
    pub fn set_confirmation_nonce(&self, ref_id: &str, nonce: &str) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE confirmations SET nonce = ?2 WHERE ref_id = ?1",
            params![ref_id, nonce],
        )?;
        if changed == 0 {
            return Err(LedgerError::NotFound(ref_id.to_string()));
        }
        Ok(())
    }

    pub fn resolve_confirmation(&self, ref_id: &str, outcome: &str) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE confirmations SET resolved_at = ?2, outcome = ?3 WHERE ref_id = ?1",
            params![ref_id, fmt_ts(now_ist()), outcome],
        )?;
        if changed == 0 {
            return Err(LedgerError::NotFound(ref_id.to_string()));
        }
        Ok(())
    }

    pub fn pending_confirm(&self, ref_id: &str) -> Result<Option<PendingConfirm>, LedgerError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT i.ref_id, i.tradingsymbol, i.side, i.quantity, i.limit_price, \
             c.requested_at, c.nonce \
             FROM confirmations c JOIN intents i ON i.ref_id = c.ref_id \
             WHERE c.ref_id = ?1 AND c.resolved_at IS NULL",
            params![ref_id],
            |row| {
                Ok(PendingConfirm {
                    ref_id: row.get(0)?,
                    tradingsymbol: row.get(1)?,
                    side: row.get(2)?,
                    quantity: row.get(3)?,
                    limit_price: row.get(4)?,
                    requested_at: row.get(5)?,
                    nonce: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(LedgerError::from)
    }

    pub fn list_pending_confirms(&self) -> Result<Vec<PendingConfirm>, LedgerError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT i.ref_id, i.tradingsymbol, i.side, i.quantity, i.limit_price, \
             c.requested_at, c.nonce \
             FROM confirmations c JOIN intents i ON i.ref_id = c.ref_id \
             WHERE c.resolved_at IS NULL ORDER BY c.requested_at ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PendingConfirm {
                    ref_id: row.get(0)?,
                    tradingsymbol: row.get(1)?,
                    side: row.get(2)?,
                    quantity: row.get(3)?,
                    limit_price: row.get(4)?,
                    requested_at: row.get(5)?,
                    nonce: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---- submissions / order status --------------------------------------

    /// Write the `submitting` row **before** the Kite call, per DESIGN.md §2.6
    /// step 6 — this is what lets a crash mid-call be recovered later.
    pub fn record_submitting(
        &self,
        ref_id: &str,
        mode: &str,
        tag: &str,
    ) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO submissions (ref_id, mode, submitting_at, broker_order_id, \
             broker_tag, accepted_at, submit_error) VALUES (?1, ?2, ?3, NULL, ?4, NULL, NULL)",
            params![ref_id, mode, fmt_ts(now_ist()), tag],
        )?;
        Ok(())
    }

    pub fn record_submission_accepted(
        &self,
        ref_id: &str,
        broker_order_id: &str,
    ) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE submissions SET broker_order_id = ?2, accepted_at = ?3 WHERE ref_id = ?1",
            params![ref_id, broker_order_id, fmt_ts(now_ist())],
        )?;
        Ok(())
    }

    pub fn record_submission_failed(&self, ref_id: &str, error: &str) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE submissions SET submit_error = ?2 WHERE ref_id = ?1",
            params![ref_id, error],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_order_status(
        &self,
        ref_id: &str,
        broker_order_id: Option<&str>,
        status: &str,
        filled_quantity: i64,
        pending_quantity: i64,
        average_price: Option<f64>,
        status_message: Option<&str>,
    ) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO order_status (ref_id, observed_at, broker_order_id, status, \
             filled_quantity, pending_quantity, average_price, status_message) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ref_id,
                fmt_ts(now_ist()),
                broker_order_id,
                status,
                filled_quantity,
                pending_quantity,
                average_price,
                status_message,
            ],
        )?;
        Ok(())
    }

    // ---- daily aggregates (derived by query, never cached) ---------------

    pub fn daily_aggregates(&self) -> Result<DailyAggregates, LedgerError> {
        let today = now_ist().format("%Y-%m-%d").to_string();
        let conn = self.lock()?;

        let orders_today: u32 = conn.query_row(
            "SELECT COUNT(DISTINCT s.ref_id) FROM submissions s \
             WHERE substr(s.submitting_at, 1, 10) = ?1",
            params![today],
            |row| row.get(0),
        )?;

        // Gross notional / new capital: sum(filled_quantity * average_price)
        // over the latest order_status row per ref_id submitted today.
        let mut stmt = conn.prepare(
            "SELECT s.ref_id, os.filled_quantity, os.average_price, i.side \
             FROM submissions s \
             JOIN intents i ON i.ref_id = s.ref_id \
             LEFT JOIN order_status os ON os.ref_id = s.ref_id \
             AND os.observed_at = ( \
               SELECT MAX(observed_at) FROM order_status WHERE ref_id = s.ref_id \
             ) \
             WHERE substr(s.submitting_at, 1, 10) = ?1",
        )?;
        let mut gross_notional_today = 0.0f64;
        let mut net_new_capital_today = 0.0f64;
        let rows = stmt.query_map(params![today], |row| {
            let filled: Option<i64> = row.get(1)?;
            let avg: Option<f64> = row.get(2)?;
            let side: String = row.get(3)?;
            Ok((filled.unwrap_or(0), avg.unwrap_or(0.0), side))
        })?;
        for r in rows {
            let (filled, avg, side) = r?;
            let notional = filled as f64 * avg;
            gross_notional_today += notional;
            if side == "BUY" {
                net_new_capital_today += notional;
            }
        }
        drop(stmt);

        let last_order_ts: Option<String> = conn
            .query_row("SELECT MAX(submitting_at) FROM submissions", [], |row| {
                row.get(0)
            })
            .optional()?
            .flatten();
        let last_order_ts = last_order_ts.and_then(|s| DateTime::parse_from_rfc3339(&s).ok());

        Ok(DailyAggregates {
            orders_today,
            gross_notional_today,
            net_new_capital_today,
            last_order_ts,
        })
    }

    // ---- kill switch / audit ----------------------------------------------

    pub fn kill_switch_engaged(&self) -> Result<bool, LedgerError> {
        let conn = self.lock()?;
        let engaged: Option<i64> = conn
            .query_row("SELECT engaged FROM kill_switch WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(engaged.unwrap_or(0) != 0)
    }

    pub fn set_kill_switch(
        &self,
        engaged: bool,
        changed_by: &str,
        note: Option<&str>,
    ) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO kill_switch (id, engaged, changed_at, changed_by, note) \
             VALUES (1, ?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET \
               engaged = excluded.engaged, changed_at = excluded.changed_at, \
               changed_by = excluded.changed_by, note = excluded.note",
            params![engaged as i64, fmt_ts(now_ist()), changed_by, note],
        )?;
        conn.execute(
            "INSERT INTO operator_audit (at, action, detail) VALUES (?1, ?2, ?3)",
            params![
                fmt_ts(now_ist()),
                if engaged { "kill" } else { "resume" },
                note,
            ],
        )?;
        Ok(())
    }

    pub fn record_audit(&self, action: &str, detail: Option<&str>) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO operator_audit (at, action, detail) VALUES (?1, ?2, ?3)",
            params![fmt_ts(now_ist()), action, detail],
        )?;
        Ok(())
    }

    pub fn record_divergence(&self, kind: &str, detail_json: &str) -> Result<(), LedgerError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO reconciliation_divergences (observed_at, kind, detail_json) \
             VALUES (?1, ?2, ?3)",
            params![fmt_ts(now_ist()), kind, detail_json],
        )?;
        Ok(())
    }

    // ---- operator reads (`tradebot status` / `tradebot ledger`) -----------

    /// Full kill-switch row, not just the boolean — `tradebot status` shows
    /// when and by whom it was last toggled.
    pub fn kill_switch(&self) -> Result<KillSwitchState, LedgerError> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT engaged, changed_at, changed_by, note FROM kill_switch WHERE id = 1",
                [],
                |row| {
                    Ok(KillSwitchState {
                        engaged: row.get::<_, i64>(0)? != 0,
                        changed_at: row.get(1)?,
                        changed_by: row.get(2)?,
                        note: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    /// The most recent reconciliation divergence, if any. Until the
    /// reconciliation loop lands this stays `None` on a healthy ledger.
    pub fn last_divergence(&self) -> Result<Option<Divergence>, LedgerError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT observed_at, kind, detail_json FROM reconciliation_divergences \
             ORDER BY observed_at DESC LIMIT 1",
            [],
            |row| {
                Ok(Divergence {
                    observed_at: row.get(0)?,
                    kind: row.get(1)?,
                    detail_json: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(LedgerError::from)
    }

    /// One intent's whole life — decision, confirmation, submission and the
    /// latest observed broker status — flattened for display.
    pub fn order(&self, ref_id: &str) -> Result<Option<OrderRow>, LedgerError> {
        let conn = self.lock()?;
        conn.query_row(
            &format!("{ORDER_ROW_SELECT} WHERE i.ref_id = ?1"),
            params![ref_id],
            order_row,
        )
        .optional()
        .map_err(LedgerError::from)
    }

    /// Every intent created today (IST), oldest first.
    pub fn orders_today(&self) -> Result<Vec<OrderRow>, LedgerError> {
        let today = now_ist().format("%Y-%m-%d").to_string();
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "{ORDER_ROW_SELECT} WHERE substr(i.created_at, 1, 10) = ?1 \
             ORDER BY i.created_at ASC"
        ))?;
        let rows = stmt
            .query_map(params![today], order_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The `limit` most recent intents, newest first.
    pub fn recent_orders(&self, limit: u32) -> Result<Vec<OrderRow>, LedgerError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "{ORDER_ROW_SELECT} ORDER BY i.created_at DESC LIMIT ?1"
        ))?;
        let rows = stmt
            .query_map(params![limit], order_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---- reconciliation (DESIGN.md §4) -----------------------------------

    /// Submissions the reconciliation pass still has work to do on: the broker
    /// may yet move them, so their state must be re-read from Kite.
    ///
    /// Two filters carry the safety reasoning:
    ///
    /// * `mode <> 'paper'` — paper submissions are filled by `PaperExecutor`
    ///   against a synthetic `PAPER-<ref_id>` id and **never appear in Kite's
    ///   order list**. Reconciling them against the broker would turn every
    ///   simulated order into a "missing broker order" divergence, so they are
    ///   excluded here rather than filtered by each caller.
    /// * latest `order_status` not terminal — [`is_terminal_status`] states are
    ///   the ones Kite will never move again, so re-reading them is pure noise.
    ///   A submission with *no* status row yet is always included: that is
    ///   exactly the crash-between-ledger-write-and-broker-call window of
    ///   DESIGN.md §8 risk 9.
    ///
    /// Oldest first, so a pass works through the backlog in the order it was
    /// created.
    pub fn open_submissions(&self) -> Result<Vec<OpenSubmission>, LedgerError> {
        // Built from TERMINAL_STATUSES so the SQL and `is_terminal_status`
        // cannot drift apart. The values are crate-owned literals, never
        // caller input, so inlining them is not an injection surface.
        let terminal = TERMINAL_STATUSES
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT s.ref_id, s.mode, s.submitting_at, s.broker_order_id, s.broker_tag, \
                    s.submit_error, os.status \
             FROM submissions s \
             LEFT JOIN order_status os ON os.ref_id = s.ref_id AND os.observed_at = ( \
                 SELECT MAX(observed_at) FROM order_status WHERE ref_id = s.ref_id) \
             WHERE s.mode <> '{MODE_PAPER}' \
               AND (os.status IS NULL OR upper(trim(os.status)) NOT IN ({terminal})) \
             ORDER BY s.submitting_at ASC"
        ))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(OpenSubmission {
                    ref_id: row.get(0)?,
                    mode: row.get(1)?,
                    submitting_at: row.get(2)?,
                    broker_order_id: row.get(3)?,
                    broker_tag: row.get(4)?,
                    submit_error: row.get(5)?,
                    latest_status: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Every broker-side identifier the ledger has ever recorded, in **all**
    /// modes and at any stage.
    ///
    /// This is the complement of [`Ledger::open_submissions`]: it answers "did
    /// *we* place this broker order?" for the divergence check in DESIGN.md §4.
    /// It deliberately includes terminal and paper rows — a broker order that
    /// matches a long-since-completed submission of ours is not a divergence,
    /// and narrowing this set would manufacture false ones.
    pub fn submission_broker_keys(&self) -> Result<Vec<SubmissionKey>, LedgerError> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT ref_id, broker_order_id, broker_tag FROM submissions")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SubmissionKey {
                    ref_id: row.get(0)?,
                    broker_order_id: row.get(1)?,
                    broker_tag: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Record a divergence only if one with the same `kind` and byte-identical
    /// `detail_json` is not already on file, returning the new row when it was
    /// written and `None` when it was a repeat.
    ///
    /// Reconciliation runs on an interval, and a genuine divergence (a broker
    /// order placed outside the bot, say) stays divergent on every subsequent
    /// pass. Appending it each time would bury the ledger in thousands of
    /// identical rows a day and drown out the *next* real divergence, so
    /// callers build a `detail_json` that is stable for a given divergent state
    /// and let this collapse the repeats. A change in that state (a foreign
    /// order moving OPEN → COMPLETE) yields different JSON and so is recorded
    /// as the new, genuinely-new observation it is.
    ///
    /// The check and the insert share one lock, so two concurrent passes cannot
    /// both decide they are the first.
    pub fn record_divergence_once(
        &self,
        kind: &str,
        detail_json: &str,
    ) -> Result<Option<Divergence>, LedgerError> {
        let conn = self.lock()?;
        let seen: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM reconciliation_divergences \
             WHERE kind = ?1 AND detail_json = ?2)",
            params![kind, detail_json],
            |row| row.get(0),
        )?;
        if seen {
            return Ok(None);
        }
        let observed_at = fmt_ts(now_ist());
        conn.execute(
            "INSERT INTO reconciliation_divergences (observed_at, kind, detail_json) \
             VALUES (?1, ?2, ?3)",
            params![observed_at, kind, detail_json],
        )?;
        Ok(Some(Divergence {
            observed_at,
            kind: kind.to_string(),
            detail_json: detail_json.to_string(),
        }))
    }

    /// The `limit` most recent divergences, newest first — what
    /// `tradebot reconcile` prints. `observed_at` is RFC-3339 at a single fixed
    /// offset, so lexicographic ordering is chronological; `rowid` breaks the
    /// tie when two rows of one pass land inside the same clock tick (Windows
    /// `Utc::now()` is only millisecond-ish), which `observed_at` alone cannot.
    pub fn list_divergences(&self, limit: u32) -> Result<Vec<Divergence>, LedgerError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT observed_at, kind, detail_json FROM reconciliation_divergences \
             ORDER BY observed_at DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(Divergence {
                    observed_at: row.get(0)?,
                    kind: row.get(1)?,
                    detail_json: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// `submissions.mode` for a `PaperExecutor` order. Paper rows are inert as far
/// as the broker is concerned — see [`Ledger::open_submissions`].
pub const MODE_PAPER: &str = "paper";

/// `order_status.status` values Kite will never move an order out of
/// (DESIGN.md §4). Everything else — OPEN, TRIGGER PENDING, the various
/// *PENDING* staging states — is still live and worth re-reading.
pub const TERMINAL_STATUSES: [&str; 3] = ["COMPLETE", "CANCELLED", "REJECTED"];

/// Whether an order has reached a state the broker will not change again.
/// Shared by `ledger`'s reconciliation query and `engine`'s reconciliation
/// pass so the two cannot disagree about what "done" means.
pub fn is_terminal_status(status: &str) -> bool {
    TERMINAL_STATUSES
        .iter()
        .any(|t| t.eq_ignore_ascii_case(status.trim()))
}

/// A submission reconciliation may still need to resolve, joined with its
/// latest observed broker status. See [`Ledger::open_submissions`].
#[derive(Debug, Clone)]
pub struct OpenSubmission {
    pub ref_id: String,
    pub mode: String,
    pub submitting_at: String,
    /// `None` once the `submitting` row was written but the broker id never
    /// came back — the re-link-by-tag case of DESIGN.md §8 risk 9.
    pub broker_order_id: Option<String>,
    /// `"tradebot:<ref_id>"`, as passed to the broker as the order tag.
    pub broker_tag: Option<String>,
    /// Already-recorded submit failure. Set means a previous pass (or the
    /// submit itself) concluded this one failed; reconciliation must not
    /// re-announce it.
    pub submit_error: Option<String>,
    pub latest_status: Option<String>,
}

/// The broker-side identifiers of one recorded submission.
/// See [`Ledger::submission_broker_keys`].
#[derive(Debug, Clone)]
pub struct SubmissionKey {
    pub ref_id: String,
    pub broker_order_id: Option<String>,
    pub broker_tag: Option<String>,
}

/// Current kill-switch row. Default (no row yet) means disengaged.
#[derive(Debug, Clone, Default)]
pub struct KillSwitchState {
    pub engaged: bool,
    pub changed_at: Option<String>,
    pub changed_by: Option<String>,
    pub note: Option<String>,
}

/// A recorded reconciliation divergence (DESIGN.md §2.8).
#[derive(Debug, Clone)]
pub struct Divergence {
    pub observed_at: String,
    pub kind: String,
    pub detail_json: String,
}

/// One intent joined with its latest decision, its confirmation, its
/// submission and its latest observed broker status. Read-only display shape
/// for the operator CLI — nothing in the write path uses it.
#[derive(Debug, Clone)]
pub struct OrderRow {
    pub ref_id: String,
    pub created_at: String,
    pub source: String,
    pub tradingsymbol: String,
    pub exchange: String,
    pub side: String,
    pub quantity: i64,
    pub order_type: String,
    pub product: String,
    pub limit_price: f64,
    pub decision: Option<String>,
    pub reasons_json: Option<String>,
    pub confirm_requested_at: Option<String>,
    pub confirm_outcome: Option<String>,
    pub mode: Option<String>,
    pub broker_order_id: Option<String>,
    pub submit_error: Option<String>,
    pub status: Option<String>,
    pub filled_quantity: Option<i64>,
    pub average_price: Option<f64>,
}

/// Shared projection behind [`Ledger::order`], [`Ledger::orders_today`] and
/// [`Ledger::recent_orders`]. `decisions` and `order_status` are append-only,
/// so both are narrowed to the newest row per `ref_id`.
const ORDER_ROW_SELECT: &str = "\
SELECT i.ref_id, i.created_at, i.source, i.tradingsymbol, i.exchange, i.side, \
       i.quantity, i.order_type, i.product, i.limit_price, \
       d.decision, d.reasons_json, c.requested_at, c.outcome, \
       s.mode, s.broker_order_id, s.submit_error, \
       os.status, os.filled_quantity, os.average_price \
FROM intents i \
LEFT JOIN decisions d ON d.ref_id = i.ref_id AND d.decided_at = ( \
    SELECT MAX(decided_at) FROM decisions WHERE ref_id = i.ref_id) \
LEFT JOIN confirmations c ON c.ref_id = i.ref_id \
LEFT JOIN submissions s ON s.ref_id = i.ref_id \
LEFT JOIN order_status os ON os.ref_id = i.ref_id AND os.observed_at = ( \
    SELECT MAX(observed_at) FROM order_status WHERE ref_id = i.ref_id)";

fn order_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrderRow> {
    Ok(OrderRow {
        ref_id: row.get(0)?,
        created_at: row.get(1)?,
        source: row.get(2)?,
        tradingsymbol: row.get(3)?,
        exchange: row.get(4)?,
        side: row.get(5)?,
        quantity: row.get(6)?,
        order_type: row.get(7)?,
        product: row.get(8)?,
        limit_price: row.get(9)?,
        decision: row.get(10)?,
        reasons_json: row.get(11)?,
        confirm_requested_at: row.get(12)?,
        confirm_outcome: row.get(13)?,
        mode: row.get(14)?,
        broker_order_id: row.get(15)?,
        submit_error: row.get(16)?,
        status: row.get(17)?,
        filled_quantity: row.get(18)?,
        average_price: row.get(19)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_intent() -> NewIntent {
        NewIntent {
            source: Source::Mcp,
            tradingsymbol: "INFY".to_string(),
            exchange: "NSE".to_string(),
            side: "BUY".to_string(),
            quantity: 1,
            order_type: "LIMIT".to_string(),
            product: "CNC".to_string(),
            limit_price: 1500.0,
        }
    }

    #[test]
    fn schema_creates_cleanly_and_is_idempotent() {
        let ledger = Ledger::open_in_memory().unwrap();
        // Re-applying the schema against the same connection must not error.
        ledger
            .conn
            .lock()
            .unwrap()
            .execute_batch(schema::SCHEMA_SQL)
            .unwrap();
    }

    #[test]
    fn record_and_read_back_intent_and_decision() {
        let ledger = Ledger::open_in_memory().unwrap();
        let hash = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.0, 0.5, 300, 1_000_000);
        let ref_id = ledger.record_intent(&sample_intent(), &hash).unwrap();
        assert!(!ref_id.is_empty());
        ledger
            .record_decision(&ref_id, "APPROVED", "v1", "[]", Some(1500.0))
            .unwrap();
    }

    #[test]
    fn duplicate_hash_detected_within_same_window() {
        let ledger = Ledger::open_in_memory().unwrap();
        let hash = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.0, 0.5, 300, 1_000_000);
        assert!(!ledger.duplicate_within_window(&hash).unwrap());
        ledger.record_intent(&sample_intent(), &hash).unwrap();
        assert!(ledger.duplicate_within_window(&hash).unwrap());
    }

    #[test]
    fn different_windows_do_not_collide() {
        let h1 = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.0, 0.5, 300, 1_000_000);
        let h2 = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.0, 0.5, 300, 1_000_301);
        assert_ne!(h1, h2);
    }

    #[test]
    fn nearby_prices_bucket_together() {
        let h1 = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.00, 0.5, 300, 1_000_000);
        let h2 = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.01, 0.5, 300, 1_000_000);
        assert_eq!(h1, h2);
    }

    #[test]
    fn kill_switch_round_trips_and_audits() {
        let ledger = Ledger::open_in_memory().unwrap();
        assert!(!ledger.kill_switch_engaged().unwrap());
        ledger
            .set_kill_switch(true, "operator", Some("test"))
            .unwrap();
        assert!(ledger.kill_switch_engaged().unwrap());
        ledger.set_kill_switch(false, "operator", None).unwrap();
        assert!(!ledger.kill_switch_engaged().unwrap());
    }

    #[test]
    fn pending_confirm_lifecycle() {
        let ledger = Ledger::open_in_memory().unwrap();
        let hash = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.0, 0.5, 300, 1_000_000);
        let ref_id = ledger.record_intent(&sample_intent(), &hash).unwrap();
        ledger
            .record_confirmation_requested(&ref_id, "telegram", Some("nonce-1"))
            .unwrap();
        assert!(ledger.pending_confirm(&ref_id).unwrap().is_some());
        assert_eq!(ledger.list_pending_confirms().unwrap().len(), 1);

        ledger.resolve_confirmation(&ref_id, "APPROVED").unwrap();
        assert!(ledger.pending_confirm(&ref_id).unwrap().is_none());
        assert_eq!(ledger.list_pending_confirms().unwrap().len(), 0);
    }

    #[test]
    fn daily_aggregates_sum_filled_notional_for_buys_only() {
        let ledger = Ledger::open_in_memory().unwrap();
        let hash = intent_hash("INFY", "BUY", 10, "LIMIT", 100.0, 0.5, 300, 1_000_000);
        let ref_id = ledger.record_intent(&sample_intent(), &hash).unwrap();
        ledger
            .record_submitting(&ref_id, "paper", "tradebot:x")
            .unwrap();
        ledger
            .record_submission_accepted(&ref_id, "broker-1")
            .unwrap();
        ledger
            .record_order_status(
                &ref_id,
                Some("broker-1"),
                "COMPLETE",
                10,
                0,
                Some(100.0),
                None,
            )
            .unwrap();

        let agg = ledger.daily_aggregates().unwrap();
        assert_eq!(agg.orders_today, 1);
        assert!((agg.gross_notional_today - 1000.0).abs() < 1e-9);
        assert!((agg.net_new_capital_today - 1000.0).abs() < 1e-9);
        assert!(agg.last_order_ts.is_some());
    }

    #[test]
    fn record_submission_failed_sets_error_without_broker_id() {
        let ledger = Ledger::open_in_memory().unwrap();
        let hash = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.0, 0.5, 300, 1_000_000);
        let ref_id = ledger.record_intent(&sample_intent(), &hash).unwrap();
        ledger
            .record_submitting(&ref_id, "live", "tradebot:x")
            .unwrap();
        ledger
            .record_submission_failed(&ref_id, "network timeout")
            .unwrap();
    }

    #[test]
    fn set_confirmation_nonce_updates_the_row() {
        let ledger = Ledger::open_in_memory().unwrap();
        let hash = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.0, 0.5, 300, 1_000_000);
        let ref_id = ledger.record_intent(&sample_intent(), &hash).unwrap();
        ledger
            .record_confirmation_requested(&ref_id, "telegram", None)
            .unwrap();
        ledger.set_confirmation_nonce(&ref_id, "abc123").unwrap();
        let pending = ledger.pending_confirm(&ref_id).unwrap().unwrap();
        assert_eq!(pending.nonce.as_deref(), Some("abc123"));
    }

    #[test]
    fn resolve_confirmation_on_unknown_ref_id_errors() {
        let ledger = Ledger::open_in_memory().unwrap();
        let err = ledger.resolve_confirmation("nope", "APPROVED").unwrap_err();
        assert!(matches!(err, LedgerError::NotFound(_)));
    }

    #[test]
    fn record_divergence_does_not_error() {
        let ledger = Ledger::open_in_memory().unwrap();
        ledger
            .record_divergence("unknown_broker_order", r#"{"order_id":"x"}"#)
            .unwrap();
    }

    #[test]
    fn kill_switch_state_reports_who_and_when() {
        let ledger = Ledger::open_in_memory().unwrap();
        let fresh = ledger.kill_switch().unwrap();
        assert!(!fresh.engaged);
        assert!(fresh.changed_at.is_none());

        ledger
            .set_kill_switch(true, "operator", Some("manual halt"))
            .unwrap();
        let engaged = ledger.kill_switch().unwrap();
        assert!(engaged.engaged);
        assert_eq!(engaged.changed_by.as_deref(), Some("operator"));
        assert_eq!(engaged.note.as_deref(), Some("manual halt"));
        assert!(engaged.changed_at.is_some());
    }

    #[test]
    fn last_divergence_is_none_until_one_is_recorded() {
        let ledger = Ledger::open_in_memory().unwrap();
        assert!(ledger.last_divergence().unwrap().is_none());
        ledger.record_divergence("unknown_order", "{}").unwrap();
        let d = ledger.last_divergence().unwrap().unwrap();
        assert_eq!(d.kind, "unknown_order");
    }

    #[test]
    fn order_row_flattens_the_whole_lifecycle() {
        let ledger = Ledger::open_in_memory().unwrap();
        let hash = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.0, 0.5, 300, 1_000_000);
        let ref_id = ledger.record_intent(&sample_intent(), &hash).unwrap();
        ledger
            .record_decision(&ref_id, "NEEDS_HUMAN_CONFIRM", "v1", "[]", Some(1499.0))
            .unwrap();
        ledger
            .record_confirmation_requested(&ref_id, "telegram", Some("nonce-1"))
            .unwrap();
        ledger.resolve_confirmation(&ref_id, "APPROVED").unwrap();
        ledger
            .record_decision(&ref_id, "APPROVED", "v1", "[]", Some(1500.0))
            .unwrap();
        ledger.record_submitting(&ref_id, "paper", "tb1").unwrap();
        ledger
            .record_submission_accepted(&ref_id, "paper-1")
            .unwrap();
        ledger
            .record_order_status(
                &ref_id,
                Some("paper-1"),
                "COMPLETE",
                1,
                0,
                Some(1500.0),
                None,
            )
            .unwrap();

        let row = ledger.order(&ref_id).unwrap().unwrap();
        assert_eq!(row.tradingsymbol, "INFY");
        assert_eq!(row.side, "BUY");
        assert_eq!(row.quantity, 1);
        assert_eq!(row.source, "mcp");
        // Latest decision wins, not the NEEDS_HUMAN_CONFIRM that preceded it.
        assert_eq!(row.decision.as_deref(), Some("APPROVED"));
        assert_eq!(row.confirm_outcome.as_deref(), Some("APPROVED"));
        assert!(row.confirm_requested_at.is_some());
        assert_eq!(row.mode.as_deref(), Some("paper"));
        assert_eq!(row.broker_order_id.as_deref(), Some("paper-1"));
        assert!(row.submit_error.is_none());
        assert_eq!(row.status.as_deref(), Some("COMPLETE"));
        assert_eq!(row.filled_quantity, Some(1));
        assert_eq!(row.average_price, Some(1500.0));
    }

    #[test]
    fn order_row_survives_an_intent_with_nothing_after_it() {
        let ledger = Ledger::open_in_memory().unwrap();
        let hash = intent_hash("INFY", "BUY", 1, "LIMIT", 1500.0, 0.5, 300, 1_000_000);
        let ref_id = ledger.record_intent(&sample_intent(), &hash).unwrap();
        let row = ledger.order(&ref_id).unwrap().unwrap();
        assert!(row.decision.is_none());
        assert!(row.confirm_outcome.is_none());
        assert!(row.status.is_none());
    }

    #[test]
    fn order_lookup_of_unknown_ref_id_is_none() {
        let ledger = Ledger::open_in_memory().unwrap();
        assert!(ledger.order("nope").unwrap().is_none());
    }

    #[test]
    fn orders_today_and_recent_orders_agree_on_a_fresh_ledger() {
        let ledger = Ledger::open_in_memory().unwrap();
        for price in [1500.0, 1600.0, 1700.0] {
            let mut intent = sample_intent();
            intent.limit_price = price;
            let hash = intent_hash("INFY", "BUY", 1, "LIMIT", price, 0.5, 300, 1_000_000);
            ledger.record_intent(&intent, &hash).unwrap();
        }
        let today = ledger.orders_today().unwrap();
        assert_eq!(today.len(), 3);
        // Oldest first for `--today`, newest first for `recent_orders`.
        let recent = ledger.recent_orders(2).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].ref_id, today[2].ref_id);
        assert_eq!(recent[1].ref_id, today[1].ref_id);
    }

    /// Record an intent plus a `submitting` row in `mode`, returning the ref_id.
    fn submitted(ledger: &Ledger, mode: &str) -> String {
        let hash = Ulid::new().to_string();
        let ref_id = ledger.record_intent(&sample_intent(), &hash).unwrap();
        let tag = format!("tradebot:{ref_id}");
        ledger.record_submitting(&ref_id, mode, &tag).unwrap();
        ref_id
    }

    #[test]
    fn terminal_statuses_are_exactly_complete_cancelled_rejected() {
        assert!(is_terminal_status("COMPLETE"));
        assert!(is_terminal_status("CANCELLED"));
        assert!(is_terminal_status("REJECTED"));
        assert!(is_terminal_status(" complete "));
        assert!(!is_terminal_status("OPEN"));
        assert!(!is_terminal_status("TRIGGER PENDING"));
        assert!(!is_terminal_status(""));
    }

    #[test]
    fn open_submissions_excludes_paper_rows() {
        let ledger = Ledger::open_in_memory().unwrap();
        let live = submitted(&ledger, "live");
        submitted(&ledger, MODE_PAPER);

        let open = ledger.open_submissions().unwrap();
        assert_eq!(open.len(), 1, "paper rows must never be reconciled");
        assert_eq!(open[0].ref_id, live);
        assert!(open[0].broker_order_id.is_none());
        assert_eq!(open[0].broker_tag, Some(format!("tradebot:{live}")));
    }

    #[test]
    fn open_submissions_excludes_terminal_but_keeps_statusless_and_open() {
        let ledger = Ledger::open_in_memory().unwrap();
        let done = submitted(&ledger, "live");
        ledger
            .record_order_status(&done, Some("b1"), "COMPLETE", 1, 0, Some(1500.0), None)
            .unwrap();
        let still_open = submitted(&ledger, "live");
        ledger
            .record_order_status(&still_open, Some("b2"), "OPEN", 0, 1, None, None)
            .unwrap();
        let no_status = submitted(&ledger, "live");

        let open = ledger.open_submissions().unwrap();
        let refs: Vec<&str> = open.iter().map(|s| s.ref_id.as_str()).collect();
        assert!(!refs.contains(&done.as_str()));
        assert!(refs.contains(&still_open.as_str()));
        assert!(refs.contains(&no_status.as_str()));
        let open_row = open.iter().find(|s| s.ref_id == still_open).unwrap();
        assert_eq!(open_row.latest_status.as_deref(), Some("OPEN"));
    }

    #[test]
    fn submission_broker_keys_includes_paper_and_terminal_rows() {
        let ledger = Ledger::open_in_memory().unwrap();
        let paper = submitted(&ledger, MODE_PAPER);
        ledger
            .record_submission_accepted(&paper, &format!("PAPER-{paper}"))
            .unwrap();
        let live = submitted(&ledger, "live");
        ledger.record_submission_accepted(&live, "250001").unwrap();

        let keys = ledger.submission_broker_keys().unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys
            .iter()
            .any(|k| k.broker_order_id.as_deref() == Some("250001")));
        assert!(keys
            .iter()
            .any(|k| k.broker_order_id.as_deref() == Some(&format!("PAPER-{paper}")[..])));
    }

    #[test]
    fn record_divergence_once_collapses_identical_repeats() {
        let ledger = Ledger::open_in_memory().unwrap();
        let detail = r#"{"broker_order_id":"250001","status":"OPEN"}"#;

        let first = ledger
            .record_divergence_once("unknown_broker_order", detail)
            .unwrap();
        assert!(first.is_some());
        let second = ledger
            .record_divergence_once("unknown_broker_order", detail)
            .unwrap();
        assert!(second.is_none(), "a repeat must not append a second row");
        assert_eq!(ledger.list_divergences(10).unwrap().len(), 1);

        // A changed state is a genuinely new observation.
        let moved = ledger
            .record_divergence_once(
                "unknown_broker_order",
                r#"{"broker_order_id":"250001","status":"COMPLETE"}"#,
            )
            .unwrap();
        assert!(moved.is_some());
        assert_eq!(ledger.list_divergences(10).unwrap().len(), 2);
    }

    #[test]
    fn list_divergences_is_newest_first_and_respects_the_limit() {
        let ledger = Ledger::open_in_memory().unwrap();
        for i in 0..3 {
            ledger
                .record_divergence_once("missing_broker_order", &format!(r#"{{"n":{i}}}"#))
                .unwrap()
                .unwrap();
        }
        let all = ledger.list_divergences(10).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].detail_json, r#"{"n":2}"#);
        assert_eq!(ledger.list_divergences(1).unwrap().len(), 1);
    }
}
