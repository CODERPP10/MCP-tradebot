//! `Engine::reconcile_once` against a real temp `Ledger` and a `wiremock` Kite
//! double — same pattern as `tests/submit.rs`.
//!
//! The ledger rows are written directly rather than through `submit`, so each
//! test can set up the exact divergent state it is about (a NULL broker id, a
//! stale OPEN status) without having to steer guardrails into producing it.

use std::sync::Arc;

use async_trait::async_trait;
use engine::Engine;
use guardrails::Policy;
use kite_client::{KiteClient, KiteConfig};
use ledger::{is_terminal_status, Ledger, NewIntent, Source};
use session::{SessionError, TokenProvider, TokenStatus};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct FixedToken;

#[async_trait]
impl TokenProvider for FixedToken {
    async fn access_token(&self) -> Result<TokenStatus, SessionError> {
        Ok(TokenStatus::Valid("tok".into()))
    }
}

fn engine_with(server: &MockServer, ledger: Arc<Ledger>) -> Engine {
    let mut cfg = KiteConfig::new("api_key_123");
    cfg.base_url = server.uri();
    let kite = KiteClient::new(cfg).unwrap();
    Engine::with_submit(
        kite,
        Arc::new(FixedToken),
        ledger,
        Arc::new(Policy::default()),
    )
}

fn new_intent() -> NewIntent {
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

/// An intent plus the `submitting` row `execute_approved` writes before the
/// broker call, in `mode`. Returns `(ref_id, tag)`.
fn submitted(ledger: &Ledger, mode: &str) -> (String, String) {
    let hash = format!("hash-{}", ulid_ish());
    let ref_id = ledger.record_intent(&new_intent(), &hash).unwrap();
    let tag = format!("tradebot:{ref_id}");
    ledger.record_submitting(&ref_id, mode, &tag).unwrap();
    (ref_id, tag)
}

/// Unique-enough suffix so repeated intents in one test do not collide on the
/// idempotency hash column.
fn ulid_ish() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    N.fetch_add(1, Ordering::Relaxed).to_string()
}

fn order_json(
    order_id: &str,
    status: &str,
    tag: Option<&str>,
    filled: i64,
    pending: i64,
    average_price: f64,
) -> serde_json::Value {
    serde_json::json!({
        "order_id": order_id,
        "status": status,
        "tradingsymbol": "INFY",
        "exchange": "NSE",
        "transaction_type": "BUY",
        "order_type": "LIMIT",
        "product": "CNC",
        "validity": "DAY",
        "quantity": 1,
        "filled_quantity": filled,
        "pending_quantity": pending,
        "price": 1500.0,
        "average_price": average_price,
        "tag": tag,
    })
}

async fn mount_orders(server: &MockServer, orders: Vec<serde_json::Value>) {
    let body = serde_json::json!({ "status": "success", "data": orders }).to_string();
    Mock::given(method("GET"))
        .and(path("/orders"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(body.into_bytes(), "application/json"),
        )
        .mount(server)
        .await;
}

/// Reconciliation must never place or cancel anything — it is a read plus
/// ledger writes. Any POST/PUT/DELETE reaching the Kite double fails the test
/// when the `MockServer` verifies its expectations on drop.
async fn forbid_mutating_calls(server: &MockServer) {
    for m in ["POST", "PUT", "DELETE"] {
        Mock::given(method(m))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .named(format!("no {m} to Kite during reconciliation"))
            .mount(server)
            .await;
    }
}

#[tokio::test]
async fn open_ledger_order_reported_complete_reaches_a_terminal_status() {
    let server = MockServer::start().await;
    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    let (ref_id, tag) = submitted(&ledger, "live");
    ledger
        .record_submission_accepted(&ref_id, "250001")
        .unwrap();
    ledger
        .record_order_status(&ref_id, Some("250001"), "OPEN", 0, 1, None, None)
        .unwrap();

    mount_orders(
        &server,
        vec![order_json("250001", "COMPLETE", Some(&tag), 1, 0, 1499.5)],
    )
    .await;
    forbid_mutating_calls(&server).await;
    let eng = engine_with(&server, ledger.clone());

    let report = eng.reconcile_once().await.unwrap();
    assert_eq!(report.ledger_submissions_examined, 1);
    assert_eq!(report.statuses_updated, 1);
    assert_eq!(report.relinked_by_tag, 0);
    assert_eq!(report.submissions_marked_failed, 0);
    assert!(report.divergences.is_empty(), "{:?}", report.divergences);

    let row = ledger.order(&ref_id).unwrap().unwrap();
    assert_eq!(row.status.as_deref(), Some("COMPLETE"));
    assert!(is_terminal_status(row.status.as_deref().unwrap()));
    assert_eq!(row.filled_quantity, Some(1));
    assert_eq!(row.average_price, Some(1499.5));

    // A terminal order is done: the next pass must not look at it again, and
    // must not mistake our own completed order for a foreign one.
    let second = eng.reconcile_once().await.unwrap();
    assert_eq!(second.ledger_submissions_examined, 0);
    assert_eq!(second.statuses_updated, 0);
    assert!(second.divergences.is_empty(), "{:?}", second.divergences);
}

#[tokio::test]
async fn unchanged_broker_status_appends_no_duplicate_row() {
    let server = MockServer::start().await;
    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    let (ref_id, tag) = submitted(&ledger, "live");
    ledger
        .record_submission_accepted(&ref_id, "250002")
        .unwrap();
    ledger
        .record_order_status(&ref_id, Some("250002"), "OPEN", 0, 1, None, None)
        .unwrap();

    mount_orders(
        &server,
        vec![order_json("250002", "OPEN", Some(&tag), 0, 1, 0.0)],
    )
    .await;
    forbid_mutating_calls(&server).await;
    let eng = engine_with(&server, ledger.clone());

    let report = eng.reconcile_once().await.unwrap();
    assert_eq!(report.ledger_submissions_examined, 1);
    assert_eq!(
        report.statuses_updated, 0,
        "a still-OPEN order must not get an identical status row every pass"
    );
    assert!(report.is_quiet());
}

#[tokio::test]
async fn broker_order_with_no_ledger_row_is_recorded_as_a_divergence() {
    let server = MockServer::start().await;
    let ledger = Arc::new(Ledger::open_in_memory().unwrap());

    // Placed by hand in Kite, or predating this ledger entirely. No tag.
    mount_orders(
        &server,
        vec![order_json("999999", "COMPLETE", None, 1, 0, 1510.0)],
    )
    .await;
    forbid_mutating_calls(&server).await;
    let eng = engine_with(&server, ledger.clone());

    let report = eng.reconcile_once().await.unwrap();
    assert_eq!(report.broker_orders_examined, 1);
    assert_eq!(report.divergences.len(), 1);
    assert_eq!(report.divergences[0].kind, "unknown_broker_order");
    assert!(report.divergences[0].detail_json.contains("999999"));
    assert_eq!(report.submissions_marked_failed, 0);

    let recorded = ledger.list_divergences(10).unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].kind, "unknown_broker_order");

    // Still divergent next pass, but already on file — reporting it again every
    // interval would bury the ledger.
    let second = eng.reconcile_once().await.unwrap();
    assert!(second.divergences.is_empty());
    assert_eq!(ledger.list_divergences(10).unwrap().len(), 1);
}

#[tokio::test]
async fn live_submitting_row_with_no_broker_order_is_marked_failed_and_never_resent() {
    let server = MockServer::start().await;
    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    // `submitting` written, broker call left no trace: no broker id, no status.
    let (ref_id, _tag) = submitted(&ledger, "live");

    mount_orders(&server, vec![]).await;
    forbid_mutating_calls(&server).await;
    let eng = engine_with(&server, ledger.clone());

    let report = eng.reconcile_once().await.unwrap();
    assert_eq!(report.ledger_submissions_examined, 1);
    assert_eq!(report.submissions_marked_failed, 1);
    assert_eq!(report.relinked_by_tag, 0);
    assert_eq!(report.divergences.len(), 1);
    assert_eq!(report.divergences[0].kind, "missing_broker_order");
    assert!(report.divergences[0].detail_json.contains(&ref_id));

    let row = ledger.order(&ref_id).unwrap().unwrap();
    assert!(
        row.submit_error.is_some(),
        "the submission must be marked submit_failed"
    );
    assert!(
        row.broker_order_id.is_none(),
        "no broker id may be invented for a submission that never reached the broker"
    );
    assert!(
        row.status.is_none(),
        "no status may be fabricated for an order the broker never acknowledged"
    );

    // Second pass: already concluded, so nothing new is said and still nothing
    // is resent (`forbid_mutating_calls` is what proves the latter).
    let second = eng.reconcile_once().await.unwrap();
    assert_eq!(second.submissions_marked_failed, 0);
    assert!(second.divergences.is_empty());
    assert_eq!(ledger.list_divergences(10).unwrap().len(), 1);
}

#[tokio::test]
async fn null_broker_id_submission_is_relinked_by_its_tradebot_tag() {
    let server = MockServer::start().await;
    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    // DESIGN.md §8 risk 9: Kite accepted the order, the process died before
    // `record_submission_accepted` ran. Only the tag connects the two.
    let (ref_id, tag) = submitted(&ledger, "live");

    mount_orders(
        &server,
        vec![order_json("250099", "OPEN", Some(&tag), 0, 1, 0.0)],
    )
    .await;
    forbid_mutating_calls(&server).await;
    let eng = engine_with(&server, ledger.clone());

    let report = eng.reconcile_once().await.unwrap();
    assert_eq!(report.relinked_by_tag, 1);
    assert_eq!(report.statuses_updated, 1);
    assert_eq!(
        report.submissions_marked_failed, 0,
        "a tag-matched order must never be treated as a submit failure"
    );
    assert!(report.divergences.is_empty(), "{:?}", report.divergences);

    let row = ledger.order(&ref_id).unwrap().unwrap();
    assert_eq!(row.broker_order_id.as_deref(), Some("250099"));
    assert!(row.submit_error.is_none());
    assert_eq!(row.status.as_deref(), Some("OPEN"));
}

#[tokio::test]
async fn paper_submissions_produce_no_divergences() {
    let server = MockServer::start().await;
    let ledger = Arc::new(Ledger::open_in_memory().unwrap());

    // (a) A paper submission stuck at `submitting` with no broker id — the
    // exact shape that reads as `missing_broker_order` for a live order.
    let (stuck, _) = submitted(&ledger, ledger::MODE_PAPER);
    // (b) A completed paper order, as `PaperExecutor` leaves it: a synthetic
    // broker id Kite has never heard of.
    let (filled, _) = submitted(&ledger, ledger::MODE_PAPER);
    let paper_id = format!("PAPER-{filled}");
    ledger
        .record_submission_accepted(&filled, &paper_id)
        .unwrap();
    ledger
        .record_order_status(
            &filled,
            Some(&paper_id),
            "COMPLETE",
            1,
            0,
            Some(1500.0),
            None,
        )
        .unwrap();

    // Kite knows about neither of them, because paper mode never calls Kite.
    mount_orders(&server, vec![]).await;
    forbid_mutating_calls(&server).await;
    let eng = engine_with(&server, ledger.clone());

    let report = eng.reconcile_once().await.unwrap();
    assert_eq!(
        report.ledger_submissions_examined, 0,
        "paper submissions must be excluded from broker status polling"
    );
    assert!(
        report.divergences.is_empty(),
        "paper orders must never read as divergences: {:?}",
        report.divergences
    );
    assert_eq!(report.submissions_marked_failed, 0);
    assert!(ledger.list_divergences(10).unwrap().is_empty());

    let row = ledger.order(&stuck).unwrap().unwrap();
    assert!(
        row.submit_error.is_none(),
        "a paper submission must not be marked submit_failed"
    );
}

#[tokio::test]
async fn reconcile_once_on_an_empty_ledger_is_a_clean_no_op() {
    let server = MockServer::start().await;
    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    mount_orders(&server, vec![]).await;
    forbid_mutating_calls(&server).await;
    let eng = engine_with(&server, ledger.clone());

    let report = eng.reconcile_once().await.unwrap();
    assert_eq!(report.broker_orders_examined, 0);
    assert_eq!(report.ledger_submissions_examined, 0);
    assert_eq!(report.statuses_updated, 0);
    assert_eq!(report.submissions_marked_failed, 0);
    assert_eq!(report.relinked_by_tag, 0);
    assert!(report.divergences.is_empty());
    assert!(report.is_quiet());
    assert!(ledger.list_divergences(10).unwrap().is_empty());
}
