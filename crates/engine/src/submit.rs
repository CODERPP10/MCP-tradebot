//! The mutating path (DESIGN.md §2.6 steps 1-6). `Engine::submit` is the only
//! place in the whole workspace that can turn an `OrderIntent` into a Kite
//! order — everything here funnels through `guardrails::evaluate` before any
//! executor runs.
//!
//! v1 scope: `PaperExecutor` only (simulated immediate fill at the limit
//! price). No live executor, no background reconciliation loop — both are
//! explicitly out of scope for this slice (see the plan / STATUS.md).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{FixedOffset, TimeZone, Utc};
use domain::{
    ConfirmOutcome, Decision, Exchange, Money, OrderIntent, OrderType, Product, Quantity, Reason,
    ReasonCode, Side, Validity, Variety,
};
use guardrails::AccountContext;
use kite_client::{Instrument, KiteError};
use ledger::{NewIntent, Source};
use tokio::sync::RwLock;

use crate::{Engine, EngineError, SubmitOutcome};

/// Instrument dump refetch interval — Kite republishes it once daily.
const INSTRUMENT_CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

pub(crate) struct InstrumentCache {
    inner: RwLock<Option<(Instant, HashMap<String, Instrument>)>>,
}

impl InstrumentCache {
    pub(crate) fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }
}

fn ist_offset() -> FixedOffset {
    FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("valid fixed offset")
}

fn money_from_f64_lossy(rupees: f64) -> Money {
    Money::from_paise((rupees * 100.0).round() as i64)
}

fn parse_side(s: &str) -> Result<Side, EngineError> {
    match s {
        "BUY" => Ok(Side::Buy),
        "SELL" => Ok(Side::Sell),
        other => Err(EngineError::InvalidIntent(format!("unknown side {other}"))),
    }
}

impl Engine {
    async fn instrument_map(&self) -> Result<HashMap<String, Instrument>, EngineError> {
        {
            let guard = self.instruments.inner.read().await;
            if let Some((fetched_at, map)) = guard.as_ref() {
                if fetched_at.elapsed() < INSTRUMENT_CACHE_TTL {
                    return Ok(map.clone());
                }
            }
        }
        let list = self
            .kite
            .instruments(Some("NSE"))
            .await
            .map_err(|e| EngineError::Kite(e.to_string()))?;
        let map: HashMap<String, Instrument> = list
            .into_iter()
            .map(|i| (i.tradingsymbol.trim().to_uppercase(), i))
            .collect();
        *self.instruments.inner.write().await = Some((Instant::now(), map.clone()));
        Ok(map)
    }

    /// Best-effort NSE holiday check: `policy.session.holidays_file` is one
    /// `YYYY-MM-DD` date per line. Missing/unreadable file is *not* treated as
    /// "no holidays" being confirmed — it just can't add information here;
    /// the session-hours check and the kill switch remain the real backstops.
    fn is_holiday(&self, today: &str) -> bool {
        std::fs::read_to_string(&self.policy.session.holidays_file)
            .map(|text| text.lines().any(|l| l.trim() == today))
            .unwrap_or(false)
    }

    async fn account_context(
        &self,
        intent: &OrderIntent,
        instrument_resolved: bool,
        duplicate_within_window: bool,
    ) -> Result<AccountContext, EngineError> {
        let now = self
            .clock
            .unwrap_or_else(|| Utc::now().with_timezone(&ist_offset()));

        let symbol = intent.tradingsymbol.trim().to_uppercase();

        let positions = self
            .kite
            .positions()
            .await
            .map_err(|e| EngineError::Kite(e.to_string()))?;
        let position_value_for_symbol = positions
            .net
            .iter()
            .find(|p| p.tradingsymbol.trim().to_uppercase() == symbol)
            .map(|p| money_from_f64_lossy((p.quantity as f64).abs() * p.average_price))
            .unwrap_or(Money::ZERO);

        let quote_result = self
            .kite
            .quote(&[format!("{}:{}", self.exchange, symbol).as_str()])
            .await;
        let (ltp, quote_age_secs, at_circuit_limit) = match quote_result {
            Ok(map) => match map.into_values().next() {
                Some(q) => {
                    // Kite's quote timestamp is naive IST ("2021-06-08
                    // 15:29:57"), no offset — parse as naive, attach IST.
                    let age = q
                        .timestamp
                        .as_deref()
                        .and_then(|t| {
                            chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S").ok()
                        })
                        .map(|naive| ist_offset().from_local_datetime(&naive).unwrap())
                        .map(|ts| (now - ts).num_seconds().max(0) as u64);
                    let circuit = (q.lower_circuit_limit > 0.0
                        && q.last_price <= q.lower_circuit_limit)
                        || (q.upper_circuit_limit > 0.0 && q.last_price >= q.upper_circuit_limit);
                    (Some(money_from_f64_lossy(q.last_price)), age, circuit)
                }
                None => (None, None, false),
            },
            // The user's Kite Connect app lacks the market-data add-on:
            // `/quote` always returns this. Fall back to the last price Kite
            // already gave us for a held/open symbol (from `positions`, just
            // fetched, or `holdings`) rather than failing every order outright
            // — but never invent a price for an unheld symbol: `ltp: None`
            // still reaches guardrails as QUOTE_STALE, per DESIGN.md §2.5
            // check 4. This is a real (if stale) price, not a bypass: the
            // price-band, cap, and confirmation checks all still run against
            // it.
            Err(KiteError::Rejected { ref code, .. }) if code == "PermissionException" => {
                let from_positions = positions
                    .net
                    .iter()
                    .find(|p| p.tradingsymbol.trim().to_uppercase() == symbol)
                    .map(|p| p.last_price);
                // Only fetched when the live quote failed — holdings is a
                // second Kite call we'd rather skip otherwise.
                let fallback = match from_positions {
                    Some(p) => Some(p),
                    None => self.kite.holdings().await.ok().and_then(|hs| {
                        hs.into_iter()
                            .find(|h| h.tradingsymbol.trim().to_uppercase() == symbol)
                            .map(|h| h.last_price)
                    }),
                };
                match fallback {
                    Some(price) => (Some(money_from_f64_lossy(price)), Some(0), false),
                    None => (None, None, false),
                }
            }
            Err(e) => return Err(EngineError::Kite(e.to_string())),
        };

        let margins = self
            .kite
            .margins_equity()
            .await
            .map_err(|e| EngineError::Kite(e.to_string()))?;
        let available_margin = money_from_f64_lossy(margins.available.live_balance);

        let agg = self.ledger.daily_aggregates().map_err(ledger_err)?;
        let seconds_since_last_order = agg
            .last_order_ts
            .map(|t| (now - t).num_seconds().max(0) as u64);

        Ok(AccountContext::new(
            self.ledger.kill_switch_engaged().map_err(ledger_err)?,
            instrument_resolved,
            ltp,
            quote_age_secs,
            at_circuit_limit,
            position_value_for_symbol,
            available_margin,
            agg.orders_today,
            money_from_f64_lossy(agg.gross_notional_today),
            money_from_f64_lossy(agg.net_new_capital_today),
            seconds_since_last_order,
            duplicate_within_window,
            now,
            self.is_holiday(&now.format("%Y-%m-%d").to_string()),
        ))
    }

    /// DESIGN.md §2.6 `submit()`, steps 1-6.
    pub async fn submit(
        &self,
        intent: OrderIntent,
        source: Source,
    ) -> Result<SubmitOutcome, EngineError> {
        self.authenticate().await?;

        // 1. Resolve + validate instrument.
        let instruments = self.instrument_map().await?;
        let key = intent.tradingsymbol.trim().to_uppercase();
        let instrument_resolved = instruments
            .get(&key)
            .map(|i| {
                self.policy
                    .instruments
                    .series
                    .iter()
                    .any(|s| s.eq_ignore_ascii_case(&i.instrument_type))
            })
            .unwrap_or(false);

        let price = intent.limit_price.map(Money::rupees_f64).unwrap_or(0.0);
        let hash = ledger::intent_hash(
            &intent.tradingsymbol,
            &intent.side.to_string(),
            i64::from(intent.quantity.get()),
            "LIMIT",
            price,
            self.policy.idempotency.price_bucket_pct,
            self.policy.idempotency.window_secs,
            Utc::now().timestamp(),
        );
        let duplicate = self
            .ledger
            .duplicate_within_window(&hash)
            .map_err(ledger_err)?;

        // 2. Reconcile context live.
        let ctx = self
            .account_context(&intent, instrument_resolved, duplicate)
            .await?;

        let ref_id = self
            .ledger
            .record_intent(
                &NewIntent {
                    source,
                    tradingsymbol: intent.tradingsymbol.clone(),
                    exchange: format!("{:?}", intent.exchange).to_uppercase(),
                    side: intent.side.to_string(),
                    quantity: i64::from(intent.quantity.get()),
                    order_type: format!("{:?}", intent.order_type).to_uppercase(),
                    product: format!("{:?}", intent.product).to_uppercase(),
                    limit_price: price,
                },
                &hash,
            )
            .map_err(ledger_err)?;

        // 3. guardrails::evaluate.
        let decision = guardrails::evaluate(&intent, &ctx, &self.policy);
        let ltp = ctx.ltp.map(Money::rupees_f64);

        match decision {
            Decision::Rejected { reasons } => {
                self.record_decision(&ref_id, "REJECTED", &reasons, ltp)?;
                Ok(SubmitOutcome::Rejected { ref_id, reasons })
            }
            Decision::NeedsHumanConfirm { .. } => {
                self.record_decision(&ref_id, "NEEDS_CONFIRM", &[], ltp)?;
                self.ledger
                    .record_confirmation_requested(&ref_id, "telegram", None)
                    .map_err(ledger_err)?;
                Ok(SubmitOutcome::Pending { ref_id })
            }
            Decision::Approved => {
                self.record_decision(&ref_id, "APPROVED", &[], ltp)?;
                self.execute_approved(&ref_id, &intent).await
            }
        }
    }

    fn record_decision(
        &self,
        ref_id: &str,
        decision: &str,
        reasons: &[Reason],
        ltp: Option<f64>,
    ) -> Result<(), EngineError> {
        let reasons_json =
            serde_json::to_string(reasons).map_err(|e| EngineError::Kite(e.to_string()))?;
        // DESIGN.md §4: the decision row records the content hash of the
        // policy that was actually in force, so the numbers behind this
        // decision can be recovered later.
        self.ledger
            .record_decision(ref_id, decision, self.policy.version(), &reasons_json, ltp)
            .map_err(ledger_err)
    }

    /// Resumes a `NeedsHumanConfirm` order once `telegram-confirm` (or the
    /// `tradebot confirm`/`reject` CLI fallback) has resolved it.
    pub async fn resume_confirmed(
        &self,
        ref_id: &str,
        outcome: ConfirmOutcome,
    ) -> Result<SubmitOutcome, EngineError> {
        match outcome {
            ConfirmOutcome::Approved => {
                let pending = self
                    .ledger
                    .pending_confirm(ref_id)
                    .map_err(ledger_err)?
                    .ok_or_else(|| EngineError::NotFound(ref_id.to_string()))?;
                self.ledger
                    .resolve_confirmation(ref_id, "APPROVED")
                    .map_err(ledger_err)?;
                let intent = OrderIntent {
                    tradingsymbol: pending.tradingsymbol,
                    exchange: Exchange::Nse,
                    side: parse_side(&pending.side)?,
                    quantity: Quantity::new(pending.quantity as u32)
                        .map_err(|e| EngineError::InvalidIntent(e.to_string()))?,
                    product: Product::Cnc,
                    variety: Variety::Regular,
                    order_type: OrderType::Limit,
                    validity: Validity::Day,
                    limit_price: Some(
                        Money::from_rupees_f64(pending.limit_price)
                            .map_err(|e| EngineError::InvalidIntent(e.to_string()))?,
                    ),
                };
                self.execute_approved(ref_id, &intent).await
            }
            ConfirmOutcome::Rejected => {
                self.ledger
                    .resolve_confirmation(ref_id, "REJECTED")
                    .map_err(ledger_err)?;
                Ok(SubmitOutcome::Rejected {
                    ref_id: ref_id.to_string(),
                    reasons: vec![Reason::new(
                        ReasonCode::ConfirmRejected,
                        "operator rejected via the confirmation channel",
                    )],
                })
            }
            ConfirmOutcome::Timeout => {
                self.ledger
                    .resolve_confirmation(ref_id, "TIMEOUT")
                    .map_err(ledger_err)?;
                Ok(SubmitOutcome::Rejected {
                    ref_id: ref_id.to_string(),
                    reasons: vec![Reason::new(
                        ReasonCode::ConfirmTimeout,
                        "confirmation window expired with no response",
                    )],
                })
            }
        }
    }

    /// Step 6: ledger `submitting` row *before* the executor runs, per
    /// DESIGN.md §2.6 and §8 risk 9 (crash recovery via reconciliation).
    async fn execute_approved(
        &self,
        ref_id: &str,
        intent: &OrderIntent,
    ) -> Result<SubmitOutcome, EngineError> {
        let tag = format!("tradebot:{ref_id}");
        self.ledger
            .record_submitting(ref_id, "paper", &tag)
            .map_err(ledger_err)?;

        // PaperExecutor: simulate an immediate fill at the limit price. No
        // Kite mutating endpoint is touched in paper mode.
        let broker_order_id = format!("PAPER-{ref_id}");
        self.ledger
            .record_submission_accepted(ref_id, &broker_order_id)
            .map_err(ledger_err)?;
        let qty = i64::from(intent.quantity.get());
        let price = intent.limit_price.map(Money::rupees_f64);
        self.ledger
            .record_order_status(
                ref_id,
                Some(&broker_order_id),
                "COMPLETE",
                qty,
                0,
                price,
                None,
            )
            .map_err(ledger_err)?;

        Ok(SubmitOutcome::Accepted {
            order_id: broker_order_id,
            ref_id: ref_id.to_string(),
        })
    }

    /// Thin: paper-mode orders fill immediately, so there is nothing open to
    /// cancel in v1. Still ledger-audited.
    pub async fn cancel(&self, order_id: &str) -> Result<SubmitOutcome, EngineError> {
        self.ledger
            .record_audit("cancel_requested", Some(order_id))
            .map_err(ledger_err)?;
        Ok(SubmitOutcome::Rejected {
            ref_id: String::new(),
            reasons: vec![Reason::new(
                ReasonCode::BrokerRejected,
                "paper mode fills orders immediately; there is nothing open to cancel",
            )],
        })
    }
}

fn ledger_err(e: ledger::LedgerError) -> EngineError {
    EngineError::Ledger(e.to_string())
}
