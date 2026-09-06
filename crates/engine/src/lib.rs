//! The single choke point (DESIGN.md §2.6).
//!
//! Stub — implemented in `feat/engine`. The only crate that calls `kite-client`
//! mutating methods. `submit()` unconditionally runs `guardrails::evaluate()`,
//! writes a `submitting` ledger row before the broker call, and never
//! auto-retries a mutating call.

use domain::OrderIntent;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine is not implemented yet (feat/engine)")]
    NotImplemented,
}

#[derive(Debug, Clone)]
pub enum SubmitOutcome {
    Accepted {
        order_id: String,
        ref_id: String,
    },
    Pending {
        ref_id: String,
    },
    Rejected {
        ref_id: String,
        reasons: Vec<domain::Reason>,
    },
}

pub async fn submit(_intent: OrderIntent) -> Result<SubmitOutcome, EngineError> {
    Err(EngineError::NotImplemented)
}
