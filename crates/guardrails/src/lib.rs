//! The deterministic, synchronous policy engine — the security-critical core.
//!
//! This crate **must not** depend on `mcp-server`, `kite-client`, or `engine`.
//! It compiles with only `domain` + serde + chrono, so there is no code path
//! from a tool to a broker order endpoint that can skip [`evaluate`]
//! (DESIGN.md §3). No network calls, no ledger access — `engine` assembles
//! every field of [`AccountContext`] from live Kite data plus ledger-derived
//! daily aggregates and passes it in by reference.

use chrono::{DateTime, Datelike, FixedOffset, NaiveTime, Timelike};

use domain::{Decision, Money, MoneyError, OrderIntent, OrderType, Reason, ReasonCode, Side};

mod policy;
pub use policy::{Policy, PolicyError};

/// Live account state assembled by `engine` (DESIGN.md §2.5). Every field is
/// a check's raw input — `evaluate` does no I/O and trusts this snapshot.
/// `#[non_exhaustive]`: construct via [`AccountContext::new`], not a literal,
/// so adding a field later does not break `engine`'s call site silently.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AccountContext {
    pub kill_switch_engaged: bool,
    /// Resolved against today's NSE instrument dump.
    pub instrument_resolved: bool,
    /// Last traded price, if a fresh-enough quote was obtainable.
    pub ltp: Option<Money>,
    pub quote_age_secs: Option<u64>,
    pub at_circuit_limit: bool,
    /// Current value of the existing position in this symbol (before this order).
    pub position_value_for_symbol: Money,
    pub available_margin: Money,
    pub orders_today: u32,
    pub gross_notional_today: Money,
    pub net_new_capital_today: Money,
    pub seconds_since_last_order: Option<u64>,
    /// `true` if `intent_hash` for this intent already exists in the ledger's
    /// current idempotency window.
    pub duplicate_within_window: bool,
    /// Current time in the session's configured timezone (IST in v1).
    pub now: DateTime<FixedOffset>,
    pub is_holiday: bool,
}

impl AccountContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kill_switch_engaged: bool,
        instrument_resolved: bool,
        ltp: Option<Money>,
        quote_age_secs: Option<u64>,
        at_circuit_limit: bool,
        position_value_for_symbol: Money,
        available_margin: Money,
        orders_today: u32,
        gross_notional_today: Money,
        net_new_capital_today: Money,
        seconds_since_last_order: Option<u64>,
        duplicate_within_window: bool,
        now: DateTime<FixedOffset>,
        is_holiday: bool,
    ) -> Self {
        Self {
            kill_switch_engaged,
            instrument_resolved,
            ltp,
            quote_age_secs,
            at_circuit_limit,
            position_value_for_symbol,
            available_margin,
            orders_today,
            gross_notional_today,
            net_new_capital_today,
            seconds_since_last_order,
            duplicate_within_window,
            now,
            is_holiday,
        }
    }
}

impl Default for AccountContext {
    /// A maximally-permissive context (fresh quote at the intent's own limit
    /// price, empty book, mid-session) — used as the base for tests, which
    /// then flip one field to exercise one check.
    fn default() -> Self {
        Self {
            kill_switch_engaged: false,
            instrument_resolved: true,
            ltp: None,
            quote_age_secs: Some(0),
            at_circuit_limit: false,
            position_value_for_symbol: Money::ZERO,
            available_margin: Money::from_rupees(1_000_000),
            orders_today: 0,
            gross_notional_today: Money::ZERO,
            net_new_capital_today: Money::ZERO,
            seconds_since_last_order: None,
            duplicate_within_window: false,
            now: midday_wednesday(),
            is_holiday: false,
        }
    }
}

/// A fixed, deterministic "safely inside market hours, not a weekend" instant
/// for tests: Wednesday 2026-03-04 11:00:00 IST.
fn midday_wednesday() -> DateTime<FixedOffset> {
    let ist = FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("valid offset");
    ist.with_ymd_and_hms(2026, 3, 4, 11, 0, 0).unwrap()
}

use chrono::TimeZone;

/// Evaluate an order intent against live context and operator policy
/// (DESIGN.md §2.5). Runs every check regardless of earlier failures so a
/// rejection carries *all* applicable reasons, not just the first.
pub fn evaluate(intent: &OrderIntent, ctx: &AccountContext, policy: &Policy) -> Decision {
    let mut reasons = Vec::new();

    // 1. Kill switch.
    if ctx.kill_switch_engaged || policy.kill_switch.force_engaged {
        reasons.push(Reason::new(ReasonCode::KillSwitch, "kill switch engaged"));
    }

    // 2. Instrument allowlist.
    let exchange_ok =
        format!("{:?}", intent.exchange).to_uppercase() == policy.instruments.exchange;
    if !exchange_ok {
        reasons.push(Reason::new(
            ReasonCode::InstrumentNotAllowed,
            format!("exchange must be {}", policy.instruments.exchange),
        ));
    }
    if !policy.instruments.symbol_allowlist.is_empty()
        && !policy
            .instruments
            .symbol_allowlist
            .iter()
            .any(|s| s.eq_ignore_ascii_case(&intent.tradingsymbol))
    {
        reasons.push(Reason::new(
            ReasonCode::InstrumentNotAllowed,
            format!("{} is not in the symbol allowlist", intent.tradingsymbol),
        ));
    }
    if policy.instruments.require_in_instrument_dump && !ctx.instrument_resolved {
        reasons.push(Reason::new(
            ReasonCode::InstrumentUnknown,
            format!(
                "{} did not resolve against today's instrument dump",
                intent.tradingsymbol
            ),
        ));
    }

    // 3. Order shape.
    let shape_ok = matches!(intent.order_type, OrderType::Limit)
        && format!("{:?}", intent.product).to_uppercase() == policy.order_shape.product
        && format!("{:?}", intent.variety).eq_ignore_ascii_case(&policy.order_shape.variety)
        && format!("{:?}", intent.validity).to_uppercase() == policy.order_shape.validity;
    if !shape_ok {
        reasons.push(Reason::new(
            ReasonCode::OrderShape,
            "order shape must be product=CNC, variety=regular, order_type=LIMIT, validity=DAY",
        ));
    }

    // Everything past this point needs a limit price; a malformed intent
    // (LIMIT with no price) already tripped `OrderShape` above and is fully
    // rejected without evaluating price/cap math against a missing number.
    let limit_price = intent.limit_price;
    if limit_price.is_none() {
        reasons.push(Reason::new(
            ReasonCode::OrderShape,
            "LIMIT orders require limit_price",
        ));
    }

    let order_value = match limit_price.and_then(|p| p.checked_mul_qty(intent.quantity.get()).ok())
    {
        Some(v) => Some(v),
        None if limit_price.is_some() => {
            reasons.push(Reason::new(
                ReasonCode::OrderValueCap,
                "order value overflowed while computing quantity * limit_price",
            ));
            None
        }
        None => None,
    };

    // 4. Price sanity (only meaningful once we have a limit price).
    if let Some(price) = limit_price {
        match ctx.ltp {
            None => reasons.push(Reason::new(
                ReasonCode::QuoteStale,
                "no live quote available",
            )),
            Some(ltp) => {
                let age_ok = ctx
                    .quote_age_secs
                    .map(|age| age <= policy.price.max_quote_age_secs)
                    .unwrap_or(false);
                if !age_ok {
                    reasons.push(Reason::new(
                        ReasonCode::QuoteStale,
                        format!(
                            "quote older than max_quote_age_secs={}",
                            policy.price.max_quote_age_secs
                        ),
                    ));
                }
                if ctx.at_circuit_limit && policy.price.reject_at_circuit_limit {
                    reasons.push(Reason::new(
                        ReasonCode::AtCircuitLimit,
                        "instrument is at its circuit limit",
                    ));
                }
                let band = ltp.rupees_f64() * policy.price.limit_price_band_pct / 100.0;
                let diff = (price.rupees_f64() - ltp.rupees_f64()).abs();
                if diff > band {
                    reasons.push(Reason::new(
                        ReasonCode::PriceOutOfBand,
                        format!(
                            "limit_price {} is outside +/-{}% of LTP {}",
                            price, policy.price.limit_price_band_pct, ltp
                        ),
                    ));
                }
            }
        }
    }

    // 5. Per-order cap.
    if let Some(value) = order_value {
        let cap = Money::from_rupees(policy.caps.max_order_value);
        if value > cap {
            reasons.push(Reason::new(
                ReasonCode::OrderValueCap,
                format!("order value {value} exceeds max_order_value {cap}"),
            ));
        }
    }

    // 6. Per-symbol position cap. A SELL reduces exposure, so only BUY adds to it.
    if let Some(value) = order_value {
        let projected = match intent.side {
            Side::Buy => ctx.position_value_for_symbol.checked_add(value),
            Side::Sell => Ok(ctx.position_value_for_symbol),
        };
        match projected {
            Ok(projected) => {
                let cap = Money::from_rupees(policy.caps.max_position_value_per_symbol);
                if projected > cap {
                    reasons.push(Reason::new(
                        ReasonCode::SymbolPositionCap,
                        format!(
                            "projected position value {projected} exceeds \
                             max_position_value_per_symbol {cap}"
                        ),
                    ));
                }
            }
            Err(MoneyError::Overflow) => reasons.push(Reason::new(
                ReasonCode::SymbolPositionCap,
                "projected position value overflowed",
            )),
            Err(e) => reasons.push(Reason::new(ReasonCode::SymbolPositionCap, e.to_string())),
        }
    }

    // 7. Daily aggregates.
    if ctx.orders_today >= policy.caps.max_orders_per_day {
        reasons.push(Reason::new(
            ReasonCode::DailyOrderCount,
            format!(
                "orders_today {} has reached max_orders_per_day {}",
                ctx.orders_today, policy.caps.max_orders_per_day
            ),
        ));
    }
    if let Some(value) = order_value {
        if let Ok(projected) = ctx.gross_notional_today.checked_add(value) {
            let cap = Money::from_rupees(policy.caps.max_gross_notional_per_day);
            if projected > cap {
                reasons.push(Reason::new(
                    ReasonCode::DailyNotionalCap,
                    format!(
                        "projected gross notional today {projected} exceeds \
                         max_gross_notional_per_day {cap}"
                    ),
                ));
            }
        }
        if matches!(intent.side, Side::Buy) {
            if let Ok(projected) = ctx.net_new_capital_today.checked_add(value) {
                let cap = Money::from_rupees(policy.caps.max_new_capital_per_day);
                if projected > cap {
                    reasons.push(Reason::new(
                        ReasonCode::DailyCapitalCap,
                        format!(
                            "projected new capital deployed today {projected} exceeds \
                             max_new_capital_per_day {cap}"
                        ),
                    ));
                }
            }
        }
    }

    // 8. Margin. CNC delivery: approximate the requirement as the order value.
    if let Some(value) = order_value {
        let buffer = Money::from_rupees(policy.margin.buffer);
        let usable = ctx
            .available_margin
            .checked_add(Money::from_paise(-buffer.paise()));
        match usable {
            Ok(usable) if value > usable => reasons.push(Reason::new(
                ReasonCode::InsufficientMargin,
                format!(
                    "order value {value} exceeds available margin {} minus buffer {buffer}",
                    ctx.available_margin
                ),
            )),
            Err(_) => reasons.push(Reason::new(
                ReasonCode::InsufficientMargin,
                "margin arithmetic overflowed",
            )),
            _ => {}
        }
    }

    // 9. Trading session.
    if !within_session(ctx.now, policy) || ctx.is_holiday {
        reasons.push(Reason::new(
            ReasonCode::MarketClosed,
            "outside the configured trading session, or an NSE holiday",
        ));
    }

    // 10. Rate limit.
    if let Some(secs) = ctx.seconds_since_last_order {
        if secs < policy.caps.min_seconds_between_orders {
            reasons.push(Reason::new(
                ReasonCode::RateLimited,
                format!(
                    "only {secs}s since the last order; \
                     min_seconds_between_orders is {}",
                    policy.caps.min_seconds_between_orders
                ),
            ));
        }
    }

    // 11. Duplicate.
    if ctx.duplicate_within_window {
        reasons.push(Reason::new(
            ReasonCode::DuplicateOrder,
            "a matching intent was already submitted within the idempotency window",
        ));
    }

    if !reasons.is_empty() {
        return Decision::Rejected { reasons };
    }

    let needs_confirm = policy.confirmation.confirm_all
        || order_value
            .map(|v| v > Money::from_rupees(policy.confirmation.confirm_above_value))
            .unwrap_or(false);
    if needs_confirm {
        return Decision::NeedsHumanConfirm { reasons: vec![] };
    }
    Decision::Approved
}

fn within_session(now: DateTime<FixedOffset>, policy: &Policy) -> bool {
    let weekday_ok = !matches!(now.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
    let Ok(open) = NaiveTime::parse_from_str(&policy.session.open, "%H:%M") else {
        return false;
    };
    let Ok(close) = NaiveTime::parse_from_str(&policy.session.close, "%H:%M") else {
        return false;
    };
    let t = NaiveTime::from_hms_opt(now.hour(), now.minute(), now.second()).unwrap_or(open);
    weekday_ok && t >= open && t <= close
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Exchange, Product, Quantity, Validity, Variety};

    fn intent() -> OrderIntent {
        OrderIntent {
            tradingsymbol: "INFY".to_string(),
            exchange: Exchange::Nse,
            side: Side::Buy,
            quantity: Quantity::new(1).unwrap(),
            product: Product::Cnc,
            variety: Variety::Regular,
            order_type: OrderType::Limit,
            validity: Validity::Day,
            limit_price: Some(Money::from_rupees(1500)),
        }
    }

    /// A context + policy that approves cleanly on its own, so each test can
    /// flip exactly one field/policy value and see exactly one reason code.
    fn happy_ctx() -> AccountContext {
        AccountContext {
            ltp: Some(Money::from_rupees(1500)),
            ..AccountContext::default()
        }
    }

    fn happy_policy() -> Policy {
        let mut p = Policy::default();
        p.confirmation.confirm_all = false;
        p.confirmation.confirm_above_value = 1_000_000;
        p
    }

    fn rejected_codes(d: Decision) -> Vec<ReasonCode> {
        match d {
            Decision::Rejected { reasons } => reasons.into_iter().map(|r| r.code).collect(),
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn happy_path_is_approved() {
        let d = evaluate(&intent(), &happy_ctx(), &happy_policy());
        assert_eq!(d, Decision::Approved);
    }

    #[test]
    fn confirm_all_routes_through_needs_human_confirm() {
        let d = evaluate(&intent(), &happy_ctx(), &Policy::default());
        assert!(matches!(d, Decision::NeedsHumanConfirm { .. }));
    }

    #[test]
    fn confirm_above_value_triggers_only_past_the_threshold() {
        let mut policy = happy_policy();
        policy.confirmation.confirm_above_value = 100; // 1 share @ 1500 > 100
        let d = evaluate(&intent(), &happy_ctx(), &policy);
        assert!(matches!(d, Decision::NeedsHumanConfirm { .. }));
    }

    #[test]
    fn check1_kill_switch_from_context() {
        let ctx = AccountContext {
            kill_switch_engaged: true,
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::KillSwitch]
        );
    }

    #[test]
    fn check1_kill_switch_from_policy_file_override() {
        let mut policy = happy_policy();
        policy.kill_switch.force_engaged = true;
        assert_eq!(
            rejected_codes(evaluate(&intent(), &happy_ctx(), &policy)),
            vec![ReasonCode::KillSwitch]
        );
    }

    #[test]
    fn check2_symbol_not_in_allowlist() {
        let mut policy = happy_policy();
        policy.instruments.symbol_allowlist = vec!["TCS".to_string()];
        assert_eq!(
            rejected_codes(evaluate(&intent(), &happy_ctx(), &policy)),
            vec![ReasonCode::InstrumentNotAllowed]
        );
    }

    #[test]
    fn check2_symbol_in_allowlist_is_case_insensitive_and_passes() {
        let mut policy = happy_policy();
        policy.instruments.symbol_allowlist = vec!["infy".to_string()];
        assert_eq!(
            evaluate(&intent(), &happy_ctx(), &policy),
            Decision::Approved
        );
    }

    #[test]
    fn check2_unresolved_instrument() {
        let ctx = AccountContext {
            instrument_resolved: false,
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::InstrumentUnknown]
        );
    }

    #[test]
    fn check3_market_order_rejected_as_order_shape() {
        let mut i = intent();
        i.order_type = OrderType::Market;
        assert_eq!(
            rejected_codes(evaluate(&i, &happy_ctx(), &happy_policy())),
            vec![ReasonCode::OrderShape]
        );
    }

    #[test]
    fn check3_mis_product_rejected_as_order_shape() {
        let mut i = intent();
        i.product = Product::Mis;
        assert_eq!(
            rejected_codes(evaluate(&i, &happy_ctx(), &happy_policy())),
            vec![ReasonCode::OrderShape]
        );
    }

    #[test]
    fn check4_no_quote_is_quote_stale() {
        let ctx = AccountContext {
            ltp: None,
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::QuoteStale]
        );
    }

    #[test]
    fn check4_stale_quote_age_rejected() {
        let ctx = AccountContext {
            ltp: Some(Money::from_rupees(1500)),
            quote_age_secs: Some(9999),
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::QuoteStale]
        );
    }

    #[test]
    fn check4_at_circuit_limit_rejected() {
        let ctx = AccountContext {
            ltp: Some(Money::from_rupees(1500)),
            at_circuit_limit: true,
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::AtCircuitLimit]
        );
    }

    #[test]
    fn check4_price_within_band_passes_and_outside_band_rejects() {
        // Default band is 3%. 1500 * 1.03 = 1545 exactly at the edge.
        let ctx = AccountContext {
            ltp: Some(Money::from_rupees(1500)),
            ..happy_ctx()
        };
        let mut i = intent();
        i.limit_price = Some(Money::from_rupees(1545));
        assert_eq!(evaluate(&i, &ctx, &happy_policy()), Decision::Approved);

        i.limit_price = Some(Money::from_rupees(1546));
        assert_eq!(
            rejected_codes(evaluate(&i, &ctx, &happy_policy())),
            vec![ReasonCode::PriceOutOfBand]
        );
    }

    #[test]
    fn check5_order_value_cap() {
        let mut policy = happy_policy();
        policy.caps.max_order_value = 1000; // 1 share @ 1500 exceeds this
        assert_eq!(
            rejected_codes(evaluate(&intent(), &happy_ctx(), &policy)),
            vec![ReasonCode::OrderValueCap]
        );
    }

    #[test]
    fn check6_symbol_position_cap_applies_to_buy_not_sell() {
        let mut policy = happy_policy();
        policy.caps.max_position_value_per_symbol = 1000;
        let ctx = happy_ctx();

        let buy = intent();
        assert_eq!(
            rejected_codes(evaluate(&buy, &ctx, &policy)),
            vec![ReasonCode::SymbolPositionCap]
        );

        let mut sell = intent();
        sell.side = Side::Sell;
        assert_eq!(evaluate(&sell, &ctx, &policy), Decision::Approved);
    }

    #[test]
    fn check7_daily_order_count_cap() {
        let mut policy = happy_policy();
        policy.caps.max_orders_per_day = 3;
        let ctx = AccountContext {
            orders_today: 3,
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &policy)),
            vec![ReasonCode::DailyOrderCount]
        );
        let ctx_ok = AccountContext {
            orders_today: 2,
            ..happy_ctx()
        };
        assert_eq!(evaluate(&intent(), &ctx_ok, &policy), Decision::Approved);
    }

    #[test]
    fn check7_daily_notional_cap() {
        let mut policy = happy_policy();
        policy.caps.max_gross_notional_per_day = 1000;
        assert_eq!(
            rejected_codes(evaluate(&intent(), &happy_ctx(), &policy)),
            vec![ReasonCode::DailyNotionalCap]
        );
    }

    #[test]
    fn check7_daily_capital_cap_only_for_buys() {
        let mut policy = happy_policy();
        policy.caps.max_new_capital_per_day = 1000;
        assert_eq!(
            rejected_codes(evaluate(&intent(), &happy_ctx(), &policy)),
            vec![ReasonCode::DailyCapitalCap]
        );
        let mut sell = intent();
        sell.side = Side::Sell;
        assert_eq!(evaluate(&sell, &happy_ctx(), &policy), Decision::Approved);
    }

    #[test]
    fn check8_insufficient_margin() {
        let ctx = AccountContext {
            available_margin: Money::from_rupees(1000),
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::InsufficientMargin]
        );
    }

    #[test]
    fn check9_weekend_is_market_closed() {
        let ist = FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
        let ctx = AccountContext {
            now: ist.with_ymd_and_hms(2026, 3, 7, 11, 0, 0).unwrap(), // Saturday
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::MarketClosed]
        );
    }

    #[test]
    fn check9_outside_hours_is_market_closed() {
        let ist = FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
        let ctx = AccountContext {
            now: ist.with_ymd_and_hms(2026, 3, 4, 16, 0, 0).unwrap(), // after 15:30
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::MarketClosed]
        );
    }

    #[test]
    fn check9_holiday_flag_is_market_closed() {
        let ctx = AccountContext {
            is_holiday: true,
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::MarketClosed]
        );
    }

    #[test]
    fn check10_rate_limited_just_under_the_gap_and_ok_right_at_it() {
        let mut policy = happy_policy();
        policy.caps.min_seconds_between_orders = 30;
        let too_soon = AccountContext {
            seconds_since_last_order: Some(29),
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &too_soon, &policy)),
            vec![ReasonCode::RateLimited]
        );
        let ok = AccountContext {
            seconds_since_last_order: Some(30),
            ..happy_ctx()
        };
        assert_eq!(evaluate(&intent(), &ok, &policy), Decision::Approved);
    }

    #[test]
    fn check11_duplicate_order() {
        let ctx = AccountContext {
            duplicate_within_window: true,
            ..happy_ctx()
        };
        assert_eq!(
            rejected_codes(evaluate(&intent(), &ctx, &happy_policy())),
            vec![ReasonCode::DuplicateOrder]
        );
    }

    #[test]
    fn multiple_failing_checks_all_surface_together() {
        let mut policy = happy_policy();
        policy.caps.max_orders_per_day = 0;
        let ctx = AccountContext {
            kill_switch_engaged: true,
            orders_today: 0,
            ..happy_ctx()
        };
        let codes = rejected_codes(evaluate(&intent(), &ctx, &policy));
        assert!(codes.contains(&ReasonCode::KillSwitch));
        assert!(codes.contains(&ReasonCode::DailyOrderCount));
    }

    #[test]
    fn stub_default_context_fails_closed_on_missing_quote() {
        // Preserves the pre-existing "fails closed with no data" behaviour.
        let decision = evaluate(&intent(), &AccountContext::default(), &Policy::default());
        assert!(matches!(decision, Decision::Rejected { .. }));
    }

    #[test]
    fn kill_switch_is_reported_distinctly() {
        let ctx = AccountContext {
            kill_switch_engaged: true,
            ..AccountContext::default()
        };
        let Decision::Rejected { reasons } = evaluate(&intent(), &ctx, &Policy::default()) else {
            panic!("expected rejection");
        };
        assert!(reasons.iter().any(|r| r.code == ReasonCode::KillSwitch));
    }
}
