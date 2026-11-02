//! `Engine::submit` end to end: all three `Decision` outcomes, against a real
//! temp `Ledger` and a `wiremock` Kite double — same pattern as `tests/reads.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{FixedOffset, TimeZone};
use domain::{Exchange, Money, OrderIntent, OrderType, Product, Quantity, Side, Validity, Variety};
use engine::{ConfirmOutcome, Engine, IntentSource, SubmitOutcome};
use guardrails::Policy;
use kite_client::{KiteClient, KiteConfig};
use ledger::Ledger;
use session::{SessionError, TokenProvider, TokenStatus};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Wednesday 2026-03-04 11:00:00 IST — safely inside market hours, matching
/// the quote timestamp `mount_common` returns, so the session-hours and
/// quote-age guardrail checks pass regardless of the real wall clock the
/// test happens to run on.
fn fixed_now() -> chrono::DateTime<FixedOffset> {
    let ist = FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
    ist.with_ymd_and_hms(2026, 3, 4, 11, 0, 0).unwrap()
}

struct FixedToken;

#[async_trait]
impl TokenProvider for FixedToken {
    async fn access_token(&self) -> Result<TokenStatus, SessionError> {
        Ok(TokenStatus::Valid("tok".into()))
    }
}

fn ok_json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_owned().into_bytes(), "application/json")
}

const INSTRUMENTS_CSV: &str = "instrument_token,exchange_token,tradingsymbol,name,last_price,expiry,strike,tick_size,lot_size,instrument_type,segment,exchange\n\
408065,1594,INFY,INFOSYS,0,,0,0.05,1,EQ,NSE,NSE\n";

async fn mount_common(server: &MockServer, last_price: f64) {
    Mock::given(method("GET"))
        .and(path("/instruments/NSE"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(INSTRUMENTS_CSV.as_bytes().to_vec(), "text/csv"),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ok_json(&format!(
            r#"{{"status":"success","data":{{"NSE:INFY":{{"instrument_token":408065,"last_price":{last_price},
                "lower_circuit_limit":0.0,"upper_circuit_limit":0.0,"timestamp":"2026-03-04 11:00:00",
                "ohlc":{{"open":1490,"high":1510,"low":1480,"close":1495}},"volume":100}}}}}}"#
        )))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/portfolio/positions"))
        .respond_with(ok_json(
            r#"{"status":"success","data":{"net":[],"day":[]}}"#,
        ))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/margins/equity"))
        .respond_with(ok_json(
            r#"{"status":"success","data":{"enabled":true,"net":1000000,
                "available":{"live_balance":1000000,"cash":1000000,"opening_balance":0,"intraday_payin":0},
                "utilised":{"debits":0,"exposure":0,"m2m_realised":0,"m2m_unrealised":0}}}"#,
        ))
        .mount(server)
        .await;
}

fn engine_with(server: &MockServer, policy: Policy) -> Engine {
    let mut cfg = KiteConfig::new("api_key_123");
    cfg.base_url = server.uri();
    let kite = KiteClient::new(cfg).unwrap();
    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    Engine::with_submit(kite, Arc::new(FixedToken), ledger, Arc::new(policy))
        .with_fixed_clock(fixed_now())
}

fn intent(limit_price: i64) -> OrderIntent {
    OrderIntent {
        tradingsymbol: "INFY".to_string(),
        exchange: Exchange::Nse,
        side: Side::Buy,
        quantity: Quantity::new(1).unwrap(),
        product: Product::Cnc,
        variety: Variety::Regular,
        order_type: OrderType::Limit,
        validity: Validity::Day,
        limit_price: Some(Money::from_rupees(limit_price)),
    }
}

fn permissive_policy() -> Policy {
    let mut p = Policy::default();
    p.confirmation.confirm_all = false;
    p.confirmation.confirm_above_value = 1_000_000;
    p
}

#[tokio::test]
async fn approved_intent_gets_a_simulated_paper_fill() {
    let server = MockServer::start().await;
    mount_common(&server, 1500.0).await;
    let eng = engine_with(&server, permissive_policy());

    let outcome = eng.submit(intent(1500), IntentSource::Mcp).await.unwrap();
    match outcome {
        SubmitOutcome::Accepted { order_id, .. } => assert!(order_id.starts_with("PAPER-")),
        other => panic!("expected Accepted, got {other:?}"),
    }
}

#[tokio::test]
async fn confirm_all_returns_pending_immediately() {
    let server = MockServer::start().await;
    mount_common(&server, 1500.0).await;
    let eng = engine_with(&server, Policy::default()); // confirm_all = true

    let outcome = eng.submit(intent(1500), IntentSource::Mcp).await.unwrap();
    assert!(matches!(outcome, SubmitOutcome::Pending { .. }));
}

#[tokio::test]
async fn price_out_of_band_is_rejected_with_reason() {
    let server = MockServer::start().await;
    mount_common(&server, 1500.0).await;
    let eng = engine_with(&server, permissive_policy());

    // default band is 3%; 1600 vs LTP 1500 is well outside it.
    let outcome = eng.submit(intent(1600), IntentSource::Mcp).await.unwrap();
    match outcome {
        SubmitOutcome::Rejected { reasons, .. } => {
            assert!(reasons
                .iter()
                .any(|r| r.code == domain::ReasonCode::PriceOutOfBand));
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test]
async fn duplicate_submission_within_window_is_rejected() {
    let server = MockServer::start().await;
    mount_common(&server, 1500.0).await;
    let eng = engine_with(&server, permissive_policy());

    let first = eng.submit(intent(1500), IntentSource::Mcp).await.unwrap();
    assert!(matches!(first, SubmitOutcome::Accepted { .. }));

    let second = eng.submit(intent(1500), IntentSource::Mcp).await.unwrap();
    match second {
        SubmitOutcome::Rejected { reasons, .. } => {
            assert!(reasons
                .iter()
                .any(|r| r.code == domain::ReasonCode::DuplicateOrder));
        }
        other => panic!("expected Rejected(DuplicateOrder), got {other:?}"),
    }
}

#[tokio::test]
async fn resume_confirmed_approved_executes_the_paper_fill() {
    let server = MockServer::start().await;
    mount_common(&server, 1500.0).await;
    let eng = engine_with(&server, Policy::default()); // confirm_all = true

    let pending = eng.submit(intent(1500), IntentSource::Mcp).await.unwrap();
    let ref_id = match pending {
        SubmitOutcome::Pending { ref_id } => ref_id,
        other => panic!("expected Pending, got {other:?}"),
    };

    let resumed = eng
        .resume_confirmed(&ref_id, ConfirmOutcome::Approved)
        .await
        .unwrap();
    assert!(matches!(resumed, SubmitOutcome::Accepted { .. }));
}

#[tokio::test]
async fn resume_confirmed_rejected_fails_closed_without_any_kite_order_call() {
    let server = MockServer::start().await;
    mount_common(&server, 1500.0).await;
    // No /orders/regular mock mounted — a place_order call would fail the
    // test with a connection/404 error, proving it was never made.
    let eng = engine_with(&server, Policy::default());

    let pending = eng.submit(intent(1500), IntentSource::Mcp).await.unwrap();
    let ref_id = match pending {
        SubmitOutcome::Pending { ref_id } => ref_id,
        other => panic!("expected Pending, got {other:?}"),
    };

    let resumed = eng
        .resume_confirmed(&ref_id, ConfirmOutcome::Rejected)
        .await
        .unwrap();
    match resumed {
        SubmitOutcome::Rejected { reasons, .. } => {
            assert!(reasons
                .iter()
                .any(|r| r.code == domain::ReasonCode::ConfirmRejected));
        }
        other => panic!("expected Rejected(ConfirmRejected), got {other:?}"),
    }
}

#[tokio::test]
async fn resume_confirmed_timeout_fails_closed() {
    let server = MockServer::start().await;
    mount_common(&server, 1500.0).await;
    let eng = engine_with(&server, Policy::default());

    let pending = eng.submit(intent(1500), IntentSource::Mcp).await.unwrap();
    let ref_id = match pending {
        SubmitOutcome::Pending { ref_id } => ref_id,
        other => panic!("expected Pending, got {other:?}"),
    };

    let resumed = eng
        .resume_confirmed(&ref_id, ConfirmOutcome::Timeout)
        .await
        .unwrap();
    match resumed {
        SubmitOutcome::Rejected { reasons, .. } => {
            assert!(reasons
                .iter()
                .any(|r| r.code == domain::ReasonCode::ConfirmTimeout));
        }
        other => panic!("expected Rejected(ConfirmTimeout), got {other:?}"),
    }
}

fn permission_denied_quote() -> ResponseTemplate {
    ResponseTemplate::new(403).set_body_raw(
        br#"{"status":"error","error_type":"PermissionException","message":"Insufficient permission for that call."}"#
            .to_vec(),
        "application/json",
    )
}

#[tokio::test]
async fn quote_permission_denied_falls_back_to_held_position_last_price() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/instruments/NSE"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(INSTRUMENTS_CSV.as_bytes().to_vec(), "text/csv"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(permission_denied_quote())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/portfolio/positions"))
        .respond_with(ok_json(
            r#"{"status":"success","data":{"net":[{"tradingsymbol":"INFY","exchange":"NSE",
                "product":"CNC","quantity":5,"average_price":1490.0,"last_price":1500.0,"pnl":50.0}],
                "day":[]}}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/margins/equity"))
        .respond_with(ok_json(
            r#"{"status":"success","data":{"enabled":true,"net":1000000,
                "available":{"live_balance":1000000,"cash":1000000,"opening_balance":0,"intraday_payin":0},
                "utilised":{"debits":0,"exposure":0,"m2m_realised":0,"m2m_unrealised":0}}}"#,
        ))
        .mount(&server)
        .await;
    let eng = engine_with(&server, permissive_policy());

    // 1500 matches the position's last_price fallback exactly, well inside band.
    let outcome = eng.submit(intent(1500), IntentSource::Mcp).await.unwrap();
    match outcome {
        SubmitOutcome::Accepted { order_id, .. } => assert!(order_id.starts_with("PAPER-")),
        other => panic!("expected Accepted (fallback price used), got {other:?}"),
    }
}

#[tokio::test]
async fn quote_permission_denied_with_no_held_position_or_holding_fails_closed_as_quote_stale() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/instruments/NSE"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(INSTRUMENTS_CSV.as_bytes().to_vec(), "text/csv"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(permission_denied_quote())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/portfolio/positions"))
        .respond_with(ok_json(
            r#"{"status":"success","data":{"net":[],"day":[]}}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/portfolio/holdings"))
        .respond_with(ok_json(r#"{"status":"success","data":[]}"#))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/margins/equity"))
        .respond_with(ok_json(
            r#"{"status":"success","data":{"enabled":true,"net":1000000,
                "available":{"live_balance":1000000,"cash":1000000,"opening_balance":0,"intraday_payin":0},
                "utilised":{"debits":0,"exposure":0,"m2m_realised":0,"m2m_unrealised":0}}}"#,
        ))
        .mount(&server)
        .await;
    let eng = engine_with(&server, permissive_policy());

    // No price available from any source — must fail closed, never silently approve.
    let outcome = eng.submit(intent(1500), IntentSource::Mcp).await.unwrap();
    match outcome {
        SubmitOutcome::Rejected { reasons, .. } => {
            assert!(reasons
                .iter()
                .any(|r| r.code == domain::ReasonCode::QuoteStale));
        }
        other => panic!("expected Rejected(QuoteStale), got {other:?}"),
    }
}

#[tokio::test]
async fn cancel_is_rejected_in_paper_mode_and_never_calls_kite() {
    let server = MockServer::start().await;
    // No mocks mounted at all: any Kite call fails the test.
    let eng = engine_with(&server, permissive_policy());

    let outcome = eng.cancel("PAPER-xyz").await.unwrap();
    assert!(matches!(outcome, SubmitOutcome::Rejected { .. }));
}
