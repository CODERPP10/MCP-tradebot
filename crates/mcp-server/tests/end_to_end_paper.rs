//! The end-to-end paper-mode integration test (roadmap item 12 /
//! `integration/mvp`).
//!
//! Everything below is driven through the **real MCP tool layer**: an `rmcp`
//! client speaking JSON-RPC over an in-process duplex pipe to the same
//! [`Tradebot`] handler `tradebot-mcp serve` mounts on stdio. That matters more
//! than convenience — DESIGN.md §3's non-negotiable is that there is no path
//! from a tool to a broker order that skips `engine::submit` →
//! `guardrails::evaluate`, so the test enters at the tool name and asserts on
//! the ledger rows that only the guardrail chain can produce.
//!
//! `mcp-server` is a bin-only crate, so the tool module is pulled in with
//! `#[path]` rather than through a lib target (adding one would change the
//! production crate's shape for a test's benefit).
//!
//! Chain exercised: tool call → `OrderIntent` → `engine::submit` →
//! `guardrails::evaluate` → ledger `intents`/`decisions`/`submissions`/
//! `order_status` rows → `PaperExecutor` fill, plus the §2.7 confirmation
//! detour and the kill switch. Paper mode throughout — there is no live
//! executor, and `--live` stays refused.

#[path = "../src/server.rs"]
mod server;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{FixedOffset, TimeZone};
use domain::ConfirmOutcome;
use engine::{Engine, SubmitOutcome};
use guardrails::Policy;
use kite_client::{KiteClient, KiteConfig};
use ledger::{Ledger, OrderRow};
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RoleServer, RunningService};
use rmcp::ServiceExt;
use serde_json::{json, Value};
use session::{SessionError, TokenProvider, TokenStatus};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use server::Tradebot;

/// Wednesday 2026-03-04 11:00:00 IST — a weekday inside the NSE session,
/// equal to the timestamp the mocked `/quote` returns, so the session-hours
/// and quote-age guardrail checks are deterministic regardless of the real
/// wall clock the test runs on.
fn fixed_now() -> chrono::DateTime<FixedOffset> {
    let ist = FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
    ist.with_ymd_and_hms(2026, 3, 4, 11, 0, 0).unwrap()
}

const LTP: f64 = 1500.0;

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

const EMPTY_POSITIONS: &str = r#"{"status":"success","data":{"net":[],"day":[]}}"#;

const HELD_POSITIONS: &str = r#"{"status":"success","data":{"net":[{"tradingsymbol":"INFY",
    "exchange":"NSE","product":"CNC","quantity":5,"average_price":1490.0,"last_price":1500.0,
    "pnl":50.0}],"day":[]}}"#;

const MARGINS: &str = r#"{"status":"success","data":{"enabled":true,"net":1000000,
    "available":{"live_balance":1000000,"cash":1000000,"opening_balance":0,"intraday_payin":0},
    "utilised":{"debits":0,"exposure":0,"m2m_realised":0,"m2m_unrealised":0}}}"#;

/// Everything `engine::submit`'s context assembly reads. Note what is
/// deliberately *absent*: no mock for Kite's `POST /orders/:variety`. Paper
/// mode must never touch a mutating endpoint, so any attempt to place a real
/// order would surface as a Kite error and fail the test.
async fn mount_kite(server: &MockServer, positions: &str) {
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
            r#"{{"status":"success","data":{{"NSE:INFY":{{"instrument_token":408065,"last_price":{LTP},
                "lower_circuit_limit":0.0,"upper_circuit_limit":0.0,"timestamp":"2026-03-04 11:00:00",
                "ohlc":{{"open":1490,"high":1510,"low":1480,"close":1495}},"volume":100}}}}}}"#
        )))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/portfolio/positions"))
        .respond_with(ok_json(positions))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/margins/equity"))
        .respond_with(ok_json(MARGINS))
        .mount(server)
        .await;
}

/// `confirm_all = false` and a confirmation threshold above anything these
/// tests order, so the confirmation detour is opt-in per scenario rather than
/// the default. Everything else keeps the DESIGN.md placeholder numbers.
fn no_confirm_policy() -> Policy {
    let mut p = Policy::default();
    p.confirmation.confirm_all = false;
    p.confirmation.confirm_above_value = 1_000_000;
    p
}

/// The real tool layer, wired to a `wiremock` Kite and a fresh in-memory
/// ledger, reachable only over MCP JSON-RPC.
struct Harness {
    /// Held so the mock Kite server (and the duplex connection) outlive the test.
    _kite: MockServer,
    _service: RunningService<RoleServer, Tradebot>,
    client: RunningService<RoleClient, ()>,
    engine: Engine,
    ledger: Arc<Ledger>,
}

impl Harness {
    async fn start(policy: Policy, positions: &str) -> Self {
        let kite_mock = MockServer::start().await;
        mount_kite(&kite_mock, positions).await;

        let mut cfg = KiteConfig::new("api_key_123");
        cfg.base_url = kite_mock.uri();
        let kite = KiteClient::new(cfg).unwrap();
        let ledger = Arc::new(Ledger::open_in_memory().unwrap());
        let engine =
            Engine::with_submit(kite, Arc::new(FixedToken), ledger.clone(), Arc::new(policy))
                .with_fixed_clock(fixed_now());

        // Both halves must run concurrently: `serve` on the server side awaits
        // the client's `initialize`, which the client sends from its own
        // `serve`.
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let handler = Tradebot::new(engine.clone());
        let serving = tokio::spawn(async move { handler.serve(server_io).await });
        let client = ().serve(client_io).await.expect("MCP client handshake");
        let service = serving.await.unwrap().expect("MCP server handshake");

        Self {
            _kite: kite_mock,
            _service: service,
            client,
            engine,
            ledger,
        }
    }

    /// One `tools/call` over the wire, with the tool's text content parsed
    /// back out as JSON (every tool in `server.rs` answers with a JSON text
    /// block).
    async fn call(&self, tool: &'static str, args: Value) -> Value {
        let arguments = match args {
            Value::Object(map) => map,
            other => panic!("tool arguments must be an object, got {other}"),
        };
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
            .await
            .unwrap_or_else(|e| panic!("tools/call {tool} failed: {e}"));
        assert_ne!(
            result.is_error,
            Some(true),
            "tools/call {tool} returned an error result: {:?}",
            result.content
        );
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_else(|| panic!("tools/call {tool} returned no text content"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("tools/call {tool} returned non-JSON text ({e}): {text}"))
    }

    /// `place_equity_order` with the shape DESIGN.md §5 defines.
    async fn place(&self, quantity: u32, limit_price: f64) -> Value {
        self.call(
            "place_equity_order",
            json!({
                "tradingsymbol": "INFY",
                "side": "BUY",
                "quantity": quantity,
                "limit_price": limit_price,
            }),
        )
        .await
    }

    fn order(&self, ref_id: &str) -> OrderRow {
        self.ledger
            .order(ref_id)
            .expect("ledger read")
            .unwrap_or_else(|| panic!("no ledger order row for ref_id {ref_id}"))
    }
}

fn reason_codes(body: &Value) -> Vec<String> {
    body["reasons"]
        .as_array()
        .unwrap_or_else(|| panic!("response has no reasons array: {body}"))
        .iter()
        .map(|r| r["code"].as_str().expect("reason.code is a string").into())
        .collect()
}

fn ref_id_of(body: &Value) -> String {
    body["ref_id"]
        .as_str()
        .unwrap_or_else(|| panic!("response has no ref_id: {body}"))
        .to_string()
}

/// Scenario 1. An in-policy limit order placed through the tool layer is
/// APPROVED, paper-fills, and leaves the full §6 ledger trail. The ledger —
/// not the tool's JSON — is the real assertion: those rows exist only if
/// `engine::submit` ran the guardrail chain and the `PaperExecutor`.
#[tokio::test]
async fn in_policy_order_is_approved_paper_filled_and_fully_journalled() {
    let h = Harness::start(no_confirm_policy(), EMPTY_POSITIONS).await;

    let body = h.place(1, LTP).await;
    assert_eq!(body["status"], "accepted", "tool response: {body}");
    let ref_id = ref_id_of(&body);
    let order_id = body["order_id"].as_str().expect("order_id").to_string();
    assert!(
        order_id.starts_with("PAPER-"),
        "paper mode must report a simulated broker id, got {order_id}"
    );
    assert!(reason_codes(&body).is_empty(), "tool response: {body}");

    // intents
    let row = h.order(&ref_id);
    assert_eq!(row.source, "mcp", "the intent must be attributed to MCP");
    assert_eq!(row.tradingsymbol, "INFY");
    assert_eq!(row.exchange, "NSE");
    assert_eq!(row.side, "BUY");
    assert_eq!(row.quantity, 1);
    assert_eq!(row.order_type, "LIMIT");
    assert_eq!(row.product, "CNC");
    assert_eq!(row.limit_price, LTP);
    // decisions
    assert_eq!(row.decision.as_deref(), Some("APPROVED"));
    assert_eq!(row.reasons_json.as_deref(), Some("[]"));
    // no confirmation detour was taken
    assert_eq!(row.confirm_requested_at, None);
    assert_eq!(row.confirm_outcome, None);
    // submissions
    assert_eq!(
        row.mode.as_deref(),
        Some("paper"),
        "the submission must be journalled as paper mode"
    );
    assert_eq!(row.broker_order_id.as_deref(), Some(order_id.as_str()));
    assert_eq!(row.submit_error, None);
    // order_status
    assert_eq!(row.status.as_deref(), Some("COMPLETE"));
    assert_eq!(
        row.filled_quantity,
        Some(1),
        "the paper fill must be for the full quantity"
    );
    assert_eq!(row.average_price, Some(LTP));

    // The same order shows up in the operator read views and the derived
    // daily counters guardrails reads back on the next order.
    let today = h.ledger.orders_today().expect("orders_today");
    assert_eq!(today.len(), 1);
    assert_eq!(today[0].ref_id, ref_id);
    let agg = h.ledger.daily_aggregates().expect("daily_aggregates");
    assert_eq!(agg.orders_today, 1);
    assert_eq!(agg.gross_notional_today, LTP);
    assert_eq!(agg.net_new_capital_today, LTP);
}

/// Scenario 2. An order over `caps.max_order_value` is rejected with that
/// specific reason code and never reaches an executor: no `submissions` row,
/// and the daily counters stay at zero.
#[tokio::test]
async fn order_over_max_order_value_is_rejected_and_never_submitted() {
    // Lift only the *other* value caps so this order trips exactly one check
    // and the assertion is about `max_order_value`, nothing else.
    let mut policy = no_confirm_policy();
    policy.caps.max_position_value_per_symbol = 10_000_000;
    policy.caps.max_gross_notional_per_day = 10_000_000;
    policy.caps.max_new_capital_per_day = 10_000_000;
    assert_eq!(policy.caps.max_order_value, 25_000);
    let h = Harness::start(policy, EMPTY_POSITIONS).await;

    // 100 x 1500 = 150,000, six times the per-order cap.
    let body = h.place(100, LTP).await;
    assert_eq!(body["status"], "rejected", "tool response: {body}");
    assert_eq!(
        reason_codes(&body),
        vec!["ORDER_VALUE_CAP"],
        "tool response: {body}"
    );
    assert!(body["order_id"].is_null(), "tool response: {body}");

    let row = h.order(&ref_id_of(&body));
    assert_eq!(row.decision.as_deref(), Some("REJECTED"));
    assert!(row
        .reasons_json
        .as_deref()
        .expect("rejection reasons are journalled")
        .contains("ORDER_VALUE_CAP"));
    assert_eq!(
        row.mode, None,
        "a rejected intent must never get a submissions row"
    );
    assert_eq!(row.broker_order_id, None);
    assert_eq!(
        row.status, None,
        "nothing was submitted, so no order status"
    );
    assert_eq!(row.filled_quantity, None);

    let agg = h.ledger.daily_aggregates().expect("daily_aggregates");
    assert_eq!(agg.orders_today, 0);
    assert_eq!(agg.gross_notional_today, 0.0);
}

/// Scenario 3a. DESIGN.md §2.7: an order that trips `NeedsHumanConfirm`
/// returns `pending_confirmation` *without blocking*, is journalled as a
/// pending confirmation, and only fills once `resume_confirmed(Approved)`
/// arrives — the path `telegram-confirm` and `tradebot confirm` drive.
#[tokio::test]
async fn order_needing_confirmation_returns_pending_then_fills_on_approval() {
    // `Policy::default()` has `confirm_all = true`.
    let h = Harness::start(Policy::default(), EMPTY_POSITIONS).await;

    // The tool must answer immediately rather than waiting on a human: a
    // blocking implementation would hang here until the timeout fires.
    let body = tokio::time::timeout(Duration::from_secs(10), h.place(1, LTP))
        .await
        .expect("place_equity_order must not block waiting for confirmation");
    assert_eq!(body["status"], "pending_confirmation", "response: {body}");
    assert!(body["order_id"].is_null(), "response: {body}");
    let ref_id = ref_id_of(&body);

    let pending = h.ledger.list_pending_confirms().expect("pending confirms");
    assert_eq!(pending.len(), 1, "the detour must be journalled");
    assert_eq!(pending[0].ref_id, ref_id);
    assert_eq!(pending[0].tradingsymbol, "INFY");
    assert_eq!(pending[0].side, "BUY");
    assert_eq!(pending[0].quantity, 1);
    assert_eq!(pending[0].limit_price, LTP);

    let row = h.order(&ref_id);
    assert_eq!(row.decision.as_deref(), Some("NEEDS_CONFIRM"));
    assert!(row.confirm_requested_at.is_some());
    assert_eq!(row.confirm_outcome, None);
    assert_eq!(
        row.mode, None,
        "nothing may be submitted while confirmation is outstanding"
    );

    let resumed = h
        .engine
        .resume_confirmed(&ref_id, ConfirmOutcome::Approved)
        .await
        .expect("resume_confirmed");
    let order_id = match resumed {
        SubmitOutcome::Accepted {
            order_id,
            ref_id: r,
        } => {
            assert_eq!(r, ref_id);
            order_id
        }
        other => panic!("expected Accepted after approval, got {other:?}"),
    };

    let row = h.order(&ref_id);
    assert_eq!(row.confirm_outcome.as_deref(), Some("APPROVED"));
    assert_eq!(row.mode.as_deref(), Some("paper"));
    assert_eq!(row.broker_order_id.as_deref(), Some(order_id.as_str()));
    assert_eq!(row.status.as_deref(), Some("COMPLETE"));
    assert_eq!(row.filled_quantity, Some(1));
    assert!(
        h.ledger
            .list_pending_confirms()
            .expect("pending confirms")
            .is_empty(),
        "an approved confirmation must no longer be pending"
    );
}

/// Scenario 3b. The other half of §2.7: a rejected confirmation resolves the
/// order with `CONFIRM_REJECTED` and produces no fill at all.
#[tokio::test]
async fn rejected_confirmation_resolves_the_order_without_any_fill() {
    let h = Harness::start(Policy::default(), EMPTY_POSITIONS).await;

    let body = h.place(1, LTP).await;
    assert_eq!(body["status"], "pending_confirmation", "response: {body}");
    let ref_id = ref_id_of(&body);

    let resumed = h
        .engine
        .resume_confirmed(&ref_id, ConfirmOutcome::Rejected)
        .await
        .expect("resume_confirmed");
    match resumed {
        SubmitOutcome::Rejected { reasons, .. } => assert!(
            reasons
                .iter()
                .any(|r| r.code == domain::ReasonCode::ConfirmRejected),
            "expected CONFIRM_REJECTED, got {reasons:?}"
        ),
        other => panic!("expected Rejected after an operator rejection, got {other:?}"),
    }

    let row = h.order(&ref_id);
    assert_eq!(row.confirm_outcome.as_deref(), Some("REJECTED"));
    assert_eq!(
        row.mode, None,
        "a rejected confirmation must never produce a submission"
    );
    assert_eq!(row.broker_order_id, None);
    assert_eq!(row.status, None);
    assert_eq!(row.filled_quantity, None);
    assert!(h
        .ledger
        .list_pending_confirms()
        .expect("pending confirms")
        .is_empty());
    let agg = h.ledger.daily_aggregates().expect("daily_aggregates");
    assert_eq!(agg.orders_today, 0);
}

/// Scenario 4. With the ledger kill switch engaged, an otherwise perfectly
/// in-policy order is rejected and nothing is submitted.
#[tokio::test]
async fn engaged_kill_switch_rejects_the_order_and_submits_nothing() {
    let h = Harness::start(no_confirm_policy(), EMPTY_POSITIONS).await;
    h.ledger
        .set_kill_switch(true, "integration-test", Some("end-to-end kill switch"))
        .expect("set_kill_switch");
    assert!(h.ledger.kill_switch_engaged().expect("kill_switch_engaged"));

    let body = h.place(1, LTP).await;
    assert_eq!(body["status"], "rejected", "tool response: {body}");
    assert!(
        reason_codes(&body).contains(&"KILL_SWITCH".to_string()),
        "tool response: {body}"
    );
    assert!(body["order_id"].is_null(), "tool response: {body}");

    let row = h.order(&ref_id_of(&body));
    assert_eq!(row.decision.as_deref(), Some("REJECTED"));
    assert!(row
        .reasons_json
        .as_deref()
        .expect("rejection reasons are journalled")
        .contains("KILL_SWITCH"));
    assert_eq!(
        row.mode, None,
        "the kill switch must stop the order before any executor runs"
    );
    assert_eq!(row.broker_order_id, None);
    assert_eq!(row.status, None);
    assert_eq!(
        h.ledger
            .daily_aggregates()
            .expect("aggregates")
            .orders_today,
        0
    );
}

/// Scenario 5. The read path is wired through the same tool layer: both
/// `get_margins` and `get_positions` return the mocked account data.
#[tokio::test]
async fn read_tools_return_the_mocked_account_data() {
    let h = Harness::start(no_confirm_policy(), HELD_POSITIONS).await;

    let margins = h.call("get_margins", json!({})).await;
    assert_eq!(margins["available"], 1_000_000.0);
    assert_eq!(margins["utilised"], 0.0);
    assert_eq!(margins["net"], 1_000_000.0);

    let positions = h.call("get_positions", json!({})).await;
    let net = positions["net"].as_array().expect("net array");
    assert_eq!(net.len(), 1, "positions: {positions}");
    assert_eq!(net[0]["tradingsymbol"], "INFY");
    assert_eq!(net[0]["quantity"], 5);
    assert_eq!(net[0]["average_price"], 1490.0);
    assert_eq!(net[0]["last_price"], LTP);
    assert!(positions["day"].as_array().expect("day array").is_empty());
}

/// DESIGN.md §2.8/§5: the advertised tool surface is exactly these eight.
/// Pinned deliberately — "no raw passthrough tool" is a non-negotiable, and a
/// new tool appearing here is a security-relevant change that should have to
/// be made consciously, not inherited from a refactor.
#[tokio::test]
async fn the_tool_surface_is_exactly_the_eight_designed_tools() {
    let h = Harness::start(no_confirm_policy(), EMPTY_POSITIONS).await;

    let names: BTreeSet<String> = h
        .client
        .list_all_tools()
        .await
        .expect("tools/list")
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    let expected: BTreeSet<String> = [
        "cancel_order",
        "get_holdings",
        "get_margins",
        "get_order_status",
        "get_positions",
        "get_quote",
        "list_orders",
        "place_equity_order",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(names, expected);
}
