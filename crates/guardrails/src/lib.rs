//! The deterministic, synchronous policy engine — the security-critical core.
//!
//! This crate **must not** depend on `mcp-server`, `kite-client`, or `engine`.
//! It compiles with only `domain` + serde, so there is no code path from a tool
//! to a broker order endpoint that can skip [`evaluate`] (DESIGN.md §3).

use domain::{Decision, OrderIntent, Reason, ReasonCode};

mod policy;
pub use policy::{Policy, PolicyError};

/// Live account state assembled by `engine` from Kite plus ledger-derived daily
/// aggregates. Passed by reference into [`evaluate`]; never fetched here.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AccountContext {
    /// Placeholder — fields (positions, margins, LTP, open orders, daily
    /// aggregates) land with `feat/engine`.
    pub kill_switch_engaged: bool,
}

/// Evaluate an order intent against live context and operator policy.
///
/// v1 stub: not implemented, so it **fails closed** — every intent is rejected
/// until `feat/guardrails` lands the real checks (DESIGN.md §2.5).
pub fn evaluate(_intent: &OrderIntent, ctx: &AccountContext, _policy: &Policy) -> Decision {
    if ctx.kill_switch_engaged {
        return Decision::Rejected {
            reasons: vec![Reason::new(ReasonCode::KillSwitch, "kill switch engaged")],
        };
    }
    Decision::Rejected {
        reasons: vec![Reason::new(
            ReasonCode::NotImplemented,
            "guardrail checks are not implemented yet (feat/guardrails)",
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Exchange, Money, OrderType, Product, Quantity, Side, Validity, Variety};

    fn sample_intent() -> OrderIntent {
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

    #[test]
    fn stub_fails_closed() {
        let decision = evaluate(
            &sample_intent(),
            &AccountContext::default(),
            &Policy::default(),
        );
        assert!(matches!(decision, Decision::Rejected { .. }));
    }

    #[test]
    fn kill_switch_is_reported_distinctly() {
        let ctx = AccountContext {
            kill_switch_engaged: true,
        };
        let Decision::Rejected { reasons } = evaluate(&sample_intent(), &ctx, &Policy::default())
        else {
            panic!("expected rejection");
        };
        assert_eq!(reasons[0].code, ReasonCode::KillSwitch);
    }
}
