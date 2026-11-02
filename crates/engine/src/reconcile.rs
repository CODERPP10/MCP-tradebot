//! Reconciliation (DESIGN.md §4): the ledger is authoritative for *what the
//! bot did*, Kite is authoritative for *what actually happened*. This module
//! walks the gap between them and writes the ledger rows that close it.
//!
//! Three invariants shape everything here:
//!
//! 1. **Nothing is ever re-sent.** A submission with no broker order is
//!    recorded as failed and left alone. "Submitted, final state unknown" is a
//!    first-class outcome (DESIGN.md §2.6, §8 risk 6) — guessing at it by
//!    resending is how one order becomes two.
//! 2. **Paper submissions are not broker orders.** `PaperExecutor` fills
//!    against a synthetic `PAPER-<ref_id>` id that Kite has never heard of, so
//!    they are excluded from both the broker status poll and the
//!    missing-broker-order check — see [`ledger::Ledger::open_submissions`].
//!    Without that exclusion every simulated order would instantly read as a
//!    divergence.
//! 3. **A broker order we did not place is reported, not adopted.** It means
//!    something else is trading the account; the operator needs to know, and
//!    nothing here tries to take ownership of it.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use kite_client::Order;

use crate::{Engine, EngineError};

/// `reconciliation_divergences.kind` for a live broker order that matches no
/// ledger submission — by id or by tag (DESIGN.md §4, §6).
///
/// Expected on any account with history older than this ledger, or that is
/// traded by hand alongside the bot: Kite's order list is account-wide, not
/// bot-scoped. That is reported rather than suppressed on purpose — the
/// operator, not this code, decides which foreign orders are benign.
const KIND_UNKNOWN_BROKER_ORDER: &str = "unknown_broker_order";

/// `reconciliation_divergences.kind` for a ledger submission with no broker
/// order behind it: the `submitting` row was written, the broker call then left
/// no trace. A silent submit failure (DESIGN.md §4).
const KIND_MISSING_BROKER_ORDER: &str = "missing_broker_order";

/// What goes in `submissions.submit_error`. Phrased so a later reader of the
/// ledger can tell this was concluded by reconciliation rather than returned by
/// Kite, and that no resend followed.
const SUBMIT_FAILED_NOTE: &str =
    "reconciliation: no broker order found for this submission; not auto-resent";

/// Outcome of one [`Engine::reconcile_once`] pass.
///
/// The counts are observations, not errors — a healthy pass on a quiet account
/// is all zeros with an empty `divergences`.
#[derive(Debug, Clone, Default)]
pub struct ReconcileReport {
    /// Orders Kite returned for the account, bot-placed or not.
    pub broker_orders_examined: usize,
    /// Non-paper, non-terminal ledger submissions considered this pass.
    pub ledger_submissions_examined: usize,
    /// New `order_status` rows appended because the broker had moved.
    pub statuses_updated: usize,
    /// Submissions marked `submit_failed`. Never accompanied by a resend.
    pub submissions_marked_failed: usize,
    /// Submissions whose broker order id was recovered from the order tag
    /// (DESIGN.md §8 risk 9).
    pub relinked_by_tag: usize,
    /// Divergences recorded **by this pass** — repeats of an already-recorded
    /// divergence are collapsed by
    /// [`ledger::Ledger::record_divergence_once`] and do not appear here, so a
    /// steady-state loop reports an empty vector rather than the same finding
    /// forever.
    pub divergences: Vec<ledger::Divergence>,
}

impl ReconcileReport {
    /// Whether the pass actually changed anything. Used to keep the background
    /// loop quiet when there is nothing to say.
    pub fn is_quiet(&self) -> bool {
        self.statuses_updated == 0
            && self.submissions_marked_failed == 0
            && self.relinked_by_tag == 0
            && self.divergences.is_empty()
    }
}

impl Engine {
    /// One reconciliation pass (DESIGN.md §4).
    ///
    /// Pulls the account's orders from Kite once, then:
    /// 1. advances every open ledger submission to the broker's current state;
    /// 2. flags broker orders with no ledger row;
    /// 3. marks ledger submissions with no broker order as failed — without
    ///    resending them;
    /// 4. re-links a submission whose broker id was lost to the order carrying
    ///    its `tradebot:<ref_id>` tag.
    ///
    /// `positions`/`margins` are deliberately *not* fetched: nothing here
    /// decides anything from them, and `submit`'s `account_context` already
    /// reads them live at the only moment they matter. Two extra Kite calls per
    /// interval, forever, for data nobody reads is not worth the rate budget.
    pub async fn reconcile_once(&self) -> Result<ReconcileReport, EngineError> {
        self.authenticate().await?;

        let broker_orders = self
            .kite
            .orders()
            .await
            .map_err(|e| EngineError::Kite(e.to_string()))?;

        let mut report = ReconcileReport {
            broker_orders_examined: broker_orders.len(),
            ..Default::default()
        };

        // Both match axes from DESIGN.md §4, built once. The tag index is what
        // makes §8 risk 9 recoverable: it holds even when the broker id never
        // made it into the ledger.
        let by_id: HashMap<&str, &Order> = broker_orders
            .iter()
            .map(|o| (o.order_id.as_str(), o))
            .collect();
        let by_tag: HashMap<&str, &Order> = broker_orders
            .iter()
            .filter_map(|o| Some((non_empty(o.tag.as_deref())?, o)))
            .collect();

        // Ledger side first — settle our own orders, then report whatever the
        // broker has left over. Neither side's correctness depends on the
        // order: `broker_tag` is written with the `submitting` row, so a
        // bot-placed order is a known key below whether or not its id was
        // recovered above. Keeping the ledger side first just means the report
        // and the stored rows describe the same end state.
        self.reconcile_ledger_side(&by_id, &by_tag, &mut report)?;
        self.reconcile_broker_side(&broker_orders, &mut report)?;

        Ok(report)
    }

    /// Steps 1, 3 and 4: ledger → broker.
    fn reconcile_ledger_side(
        &self,
        by_id: &HashMap<&str, &Order>,
        by_tag: &HashMap<&str, &Order>,
        report: &mut ReconcileReport,
    ) -> Result<(), EngineError> {
        let open = self.ledger.open_submissions().map_err(ledger_err)?;
        report.ledger_submissions_examined = open.len();

        for sub in &open {
            // Broker order id first (authoritative), tag second (recovery).
            let matched = non_empty(sub.broker_order_id.as_deref())
                .and_then(|id| by_id.get(id).copied())
                .or_else(|| {
                    non_empty(sub.broker_tag.as_deref()).and_then(|tag| by_tag.get(tag).copied())
                });

            let Some(order) = matched else {
                // Step 3. `submit_error` already set means an earlier pass (or
                // the submit itself) reached this conclusion; re-recording it
                // would just churn the ledger. Either way: no resend.
                if sub.submit_error.is_none() {
                    self.ledger
                        .record_submission_failed(&sub.ref_id, SUBMIT_FAILED_NOTE)
                        .map_err(ledger_err)?;
                    report.submissions_marked_failed += 1;

                    let detail = serde_json::json!({
                        "ref_id": sub.ref_id,
                        "mode": sub.mode,
                        "submitting_at": sub.submitting_at,
                        "broker_order_id": sub.broker_order_id,
                        "broker_tag": sub.broker_tag,
                        "latest_status": sub.latest_status,
                        "action": "marked submit_failed; not auto-resent",
                    })
                    .to_string();
                    self.record_divergence(KIND_MISSING_BROKER_ORDER, &detail, report)?;
                }
                continue;
            };

            // Step 4 (DESIGN.md §8 risk 9): the broker accepted it, the ledger
            // never learned the id. Recovering it from the tag is what keeps
            // this out of the submit-failure branch above.
            if non_empty(sub.broker_order_id.as_deref()).is_none() {
                self.ledger
                    .record_submission_accepted(&sub.ref_id, &order.order_id)
                    .map_err(ledger_err)?;
                report.relinked_by_tag += 1;
            }

            // Step 1. `order_status` is append-only, so only write when the
            // broker has actually moved — otherwise every interval would add a
            // row saying "still OPEN".
            if sub.latest_status.as_deref() != Some(order.status.as_str()) {
                self.ledger
                    .record_order_status(
                        &sub.ref_id,
                        Some(&order.order_id),
                        &order.status,
                        order.filled_quantity,
                        order.pending_quantity,
                        // Kite reports 0.0 for an unfilled order; a real
                        // average price of zero does not exist, so NULL is the
                        // honest column value.
                        (order.average_price > 0.0).then_some(order.average_price),
                        order.status_message.as_deref(),
                    )
                    .map_err(ledger_err)?;
                report.statuses_updated += 1;
            }
        }
        Ok(())
    }

    /// Step 2: broker → ledger. A broker order matching no submission of ours,
    /// on either axis, means something else is trading this account.
    fn reconcile_broker_side(
        &self,
        broker_orders: &[Order],
        report: &mut ReconcileReport,
    ) -> Result<(), EngineError> {
        if broker_orders.is_empty() {
            return Ok(());
        }
        // All modes, all stages — a paper row or a long-completed live one is
        // still proof we placed that order, and narrowing this set would
        // manufacture false divergences.
        let keys = self.ledger.submission_broker_keys().map_err(ledger_err)?;
        let known_ids: HashSet<&str> = keys
            .iter()
            .filter_map(|k| non_empty(k.broker_order_id.as_deref()))
            .collect();
        let known_tags: HashSet<&str> = keys
            .iter()
            .filter_map(|k| non_empty(k.broker_tag.as_deref()))
            .collect();

        for order in broker_orders {
            let ours = known_ids.contains(order.order_id.as_str())
                || non_empty(order.tag.as_deref()).is_some_and(|t| known_tags.contains(t));
            if ours {
                continue;
            }
            // Stable for a given (order, state) so the loop does not re-record
            // the same foreign order every interval — but a state change is a
            // new observation and is recorded.
            let detail = serde_json::json!({
                "broker_order_id": order.order_id,
                "tradingsymbol": order.tradingsymbol,
                "transaction_type": order.transaction_type,
                "quantity": order.quantity,
                "filled_quantity": order.filled_quantity,
                "status": order.status,
                "tag": order.tag,
            })
            .to_string();
            self.record_divergence(KIND_UNKNOWN_BROKER_ORDER, &detail, report)?;
        }
        Ok(())
    }

    fn record_divergence(
        &self,
        kind: &str,
        detail_json: &str,
        report: &mut ReconcileReport,
    ) -> Result<(), EngineError> {
        if let Some(d) = self
            .ledger
            .record_divergence_once(kind, detail_json)
            .map_err(ledger_err)?
        {
            report.divergences.push(d);
        }
        Ok(())
    }

    /// Run [`Engine::reconcile_once`] forever, `interval` apart.
    ///
    /// Never returns and never propagates a failure: a stale token, a Kite
    /// 5xx, or a dropped connection is logged and retried on the next tick.
    /// Reconciliation is a repair mechanism — a loop that dies on the first
    /// transient error would leave the ledger silently drifting from the broker
    /// exactly when it matters most. It mutates nothing at the broker, so
    /// retrying costs nothing but a read.
    ///
    /// The caller owns the task: `mcp-server` spawns this and drops/aborts the
    /// handle at shutdown. It does not spawn itself, so a test can simply not
    /// call it.
    ///
    /// Logging goes to stderr via `eprintln!`, matching the rest of the
    /// workspace — stdout is the MCP stdio protocol channel and must never
    /// carry diagnostics.
    pub async fn reconcile_loop(&self, interval: Duration) -> ! {
        loop {
            match self.reconcile_once().await {
                Ok(report) => {
                    if !report.is_quiet() {
                        eprintln!(
                            "tradebot: reconciled {} broker order(s) / {} open submission(s): \
                             {} status update(s), {} marked submit_failed, {} re-linked by tag, \
                             {} new divergence(s)",
                            report.broker_orders_examined,
                            report.ledger_submissions_examined,
                            report.statuses_updated,
                            report.submissions_marked_failed,
                            report.relinked_by_tag,
                            report.divergences.len(),
                        );
                        for d in &report.divergences {
                            eprintln!("tradebot: divergence {}: {}", d.kind, d.detail_json);
                        }
                    }
                }
                Err(e) => eprintln!(
                    "tradebot: reconciliation pass failed, retrying in {}s: {e}",
                    interval.as_secs()
                ),
            }
            tokio::time::sleep(interval).await;
        }
    }
}

/// Mirrors `submit.rs`'s helper of the same name — `LedgerError` is a storage
/// detail that should not leak out of the crate's public API.
fn ledger_err(e: ledger::LedgerError) -> EngineError {
    EngineError::Ledger(e.to_string())
}

/// `Some(trimmed)` for a present, non-blank string. Kite returns `""` for an
/// absent tag about as often as it omits the field, and the ledger stores
/// whatever it was given — treating `""` as a match would link unrelated rows.
fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}
