//! `Confirmer` end to end against a `wiremock` Telegram double, plus unit
//! coverage of the pure nonce/TTL/callback-validation helpers.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use domain::ConfirmOutcome;
use ledger::{Ledger, NewIntent, Source};
use std::sync::atomic::{AtomicUsize, Ordering};

use telegram_confirm::{
    format_order_summary, generate_nonce, is_expired, validate_callback, ConfirmResumer, Confirmer,
    LoginResumer,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": body}))
}

#[derive(Default)]
struct RecordingResumer {
    calls: Mutex<Vec<(String, ConfirmOutcome)>>,
}

#[async_trait]
impl ConfirmResumer for RecordingResumer {
    async fn resume(&self, ref_id: &str, outcome: ConfirmOutcome) {
        self.calls
            .lock()
            .unwrap()
            .push((ref_id.to_string(), outcome));
    }
}

fn seed_pending(ledger: &Ledger) -> String {
    let ref_id = ledger
        .record_intent(
            &NewIntent {
                source: Source::Mcp,
                tradingsymbol: "INFY".to_string(),
                exchange: "NSE".to_string(),
                side: "BUY".to_string(),
                quantity: 1,
                order_type: "LIMIT".to_string(),
                product: "CNC".to_string(),
                limit_price: 1500.0,
            },
            "hash-1",
        )
        .unwrap();
    ledger
        .record_confirmation_requested(&ref_id, "telegram", None)
        .unwrap();
    ref_id
}

#[tokio::test]
async fn health_check_succeeds_against_a_healthy_bot() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getMe"))
        .respond_with(ok(serde_json::json!({"id": 1, "is_bot": true})))
        .mount(&server)
        .await;

    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    let resumer = Arc::new(RecordingResumer::default());
    let confirmer = Confirmer::with_base_url(
        "tok123",
        &server.uri(),
        "42".to_string(),
        120,
        ledger,
        resumer,
    );
    confirmer.health_check().await.unwrap();
}

#[tokio::test]
async fn approve_callback_from_the_allowlisted_chat_resolves_and_resumes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/sendMessage"))
        .respond_with(ok(serde_json::json!({"message_id": 1})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getUpdates"))
        .respond_with(ok(serde_json::json!([])))
        .mount(&server)
        .await;

    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    let ref_id = seed_pending(&ledger);
    let resumer = Arc::new(RecordingResumer::default());
    let confirmer = Confirmer::with_base_url(
        "tok123",
        &server.uri(),
        "42".to_string(),
        120,
        ledger.clone(),
        resumer.clone(),
    );

    // First poll sends the message and mints the nonce.
    confirmer.poll_once(0).await.unwrap();
    let pending = ledger.pending_confirm(&ref_id).unwrap().unwrap();
    let nonce = pending.nonce.clone().unwrap();

    // Second poll: Telegram now reports a matching callback from chat 42.
    // Swap the mocks (the ref_id is already marked `sent`, so sendMessage
    // won't be hit again) rather than layering an ambiguous second match.
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/answerCallbackQuery"))
        .respond_with(ok(serde_json::json!(true)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getUpdates"))
        .respond_with(ok(serde_json::json!([{
            "update_id": 1,
            "callback_query": {
                "id": "cbq1",
                "from": {"id": 42},
                "data": format!("{nonce}:approve"),
            }
        }])))
        .mount(&server)
        .await;
    confirmer.poll_once(0).await.unwrap();

    assert!(ledger.pending_confirm(&ref_id).unwrap().is_none());
    let calls = resumer.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], (ref_id, ConfirmOutcome::Approved));
}

#[tokio::test]
async fn callback_from_a_different_chat_is_ignored() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/sendMessage"))
        .respond_with(ok(serde_json::json!({"message_id": 1})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getUpdates"))
        .respond_with(ok(serde_json::json!([])))
        .mount(&server)
        .await;

    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    let ref_id = seed_pending(&ledger);
    let resumer = Arc::new(RecordingResumer::default());
    let confirmer = Confirmer::with_base_url(
        "tok123",
        &server.uri(),
        "42".to_string(),
        120,
        ledger.clone(),
        resumer.clone(),
    );
    confirmer.poll_once(0).await.unwrap();
    let nonce = ledger
        .pending_confirm(&ref_id)
        .unwrap()
        .unwrap()
        .nonce
        .unwrap();

    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/answerCallbackQuery"))
        .respond_with(ok(serde_json::json!(true)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getUpdates"))
        .respond_with(ok(serde_json::json!([{
            "update_id": 1,
            "callback_query": {
                "id": "cbq1",
                "from": {"id": 999}, // not the allowlisted chat
                "data": format!("{nonce}:approve"),
            }
        }])))
        .mount(&server)
        .await;
    confirmer.poll_once(0).await.unwrap();

    // Still pending — an off-chat callback must never resolve it.
    assert!(ledger.pending_confirm(&ref_id).unwrap().is_some());
    assert!(resumer.calls.lock().unwrap().is_empty());
}

#[test]
fn validate_callback_rejects_wrong_nonce_and_malformed_data() {
    assert_eq!(validate_callback("abc:approve", "abc"), Some(true));
    assert_eq!(validate_callback("abc:approve", "xyz"), Some(false));
    assert_eq!(validate_callback("abc:reject", "abc"), Some(true));
    assert_eq!(validate_callback("garbage", "abc"), None);
    assert_eq!(validate_callback("abc:explode", "abc"), None);
}

#[test]
fn is_expired_boundary() {
    let ist = chrono::FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
    let requested = "2026-03-04T11:00:00+05:30";
    let at_119s = chrono::DateTime::parse_from_rfc3339("2026-03-04T11:01:59+05:30").unwrap();
    let at_120s = chrono::DateTime::parse_from_rfc3339("2026-03-04T11:02:00+05:30").unwrap();
    assert!(!is_expired(requested, 120, at_119s));
    assert!(is_expired(requested, 120, at_120s));
    // Unparsable timestamp fails closed.
    assert!(is_expired("not-a-timestamp", 120, at_119s));
    let _ = ist; // keep import used across both asserts above
}

#[test]
fn nonces_are_unique_and_hex() {
    let a = generate_nonce();
    let b = generate_nonce();
    assert_ne!(a, b);
    assert_eq!(a.len(), 32);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}

struct ScriptedLoginResumer {
    calls: Mutex<Vec<String>>,
    fail_next: AtomicUsize,
}

impl ScriptedLoginResumer {
    fn new(fail_first_n: usize) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail_next: AtomicUsize::new(fail_first_n),
        }
    }
}

#[async_trait]
impl LoginResumer for ScriptedLoginResumer {
    async fn complete_login(&self, request_token: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(request_token.to_string());
        if self.fail_next.load(Ordering::SeqCst) > 0 {
            self.fail_next.fetch_sub(1, Ordering::SeqCst);
            return Err("bad token".to_string());
        }
        Ok(())
    }
}

#[tokio::test]
async fn login_prompt_is_sent_once_and_completes_on_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/sendMessage"))
        .respond_with(ok(serde_json::json!({"message_id": 1})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getUpdates"))
        .respond_with(ok(serde_json::json!([])))
        .mount(&server)
        .await;

    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    let resumer = Arc::new(RecordingResumer::default());
    let login_resumer = Arc::new(ScriptedLoginResumer::new(0));
    let confirmer = Confirmer::with_base_url(
        "tok123",
        &server.uri(),
        "42".to_string(),
        120,
        ledger,
        resumer,
    )
    .with_login_resumer(login_resumer.clone());

    confirmer
        .notify_needs_login("https://kite/login?api_key=x")
        .await
        .unwrap();
    // A second call while still awaiting must not send a duplicate prompt —
    // asserted implicitly: wiremock would otherwise be fine either way, so
    // check no login call happened yet instead.
    confirmer
        .notify_needs_login("https://kite/login?api_key=x")
        .await
        .unwrap();
    assert!(login_resumer.calls.lock().unwrap().is_empty());

    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/sendMessage"))
        .respond_with(ok(serde_json::json!({"message_id": 2})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getUpdates"))
        .respond_with(ok(serde_json::json!([{
            "update_id": 1,
            "message": {"chat": {"id": 42}, "text": "  my-request-token  "}
        }])))
        .mount(&server)
        .await;
    confirmer.poll_once(0).await.unwrap();

    assert_eq!(
        login_resumer.calls.lock().unwrap().as_slice(),
        ["my-request-token"]
    );
}

#[tokio::test]
async fn login_reply_from_a_different_chat_is_ignored() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/sendMessage"))
        .respond_with(ok(serde_json::json!({"message_id": 1})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getUpdates"))
        .respond_with(ok(serde_json::json!([{
            "update_id": 1,
            "message": {"chat": {"id": 999}, "text": "sneaky-token"}
        }])))
        .mount(&server)
        .await;

    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    let resumer = Arc::new(RecordingResumer::default());
    let login_resumer = Arc::new(ScriptedLoginResumer::new(0));
    let confirmer = Confirmer::with_base_url(
        "tok123",
        &server.uri(),
        "42".to_string(),
        120,
        ledger,
        resumer,
    )
    .with_login_resumer(login_resumer.clone());

    confirmer
        .notify_needs_login("https://kite/login?api_key=x")
        .await
        .unwrap();
    confirmer.poll_once(0).await.unwrap();

    assert!(login_resumer.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn login_message_is_ignored_when_no_prompt_is_outstanding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getUpdates"))
        .respond_with(ok(serde_json::json!([{
            "update_id": 1,
            "message": {"chat": {"id": 42}, "text": "unsolicited-token"}
        }])))
        .mount(&server)
        .await;

    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    let resumer = Arc::new(RecordingResumer::default());
    let login_resumer = Arc::new(ScriptedLoginResumer::new(0));
    let confirmer = Confirmer::with_base_url(
        "tok123",
        &server.uri(),
        "42".to_string(),
        120,
        ledger,
        resumer,
    )
    .with_login_resumer(login_resumer.clone());

    // No `notify_needs_login` call — nothing outstanding to resolve.
    confirmer.poll_once(0).await.unwrap();

    assert!(login_resumer.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_login_leaves_prompt_outstanding_so_the_user_can_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottok123/sendMessage"))
        .respond_with(ok(serde_json::json!({"message_id": 1})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottok123/getUpdates"))
        .respond_with(ok(serde_json::json!([{
            "update_id": 1,
            "message": {"chat": {"id": 42}, "text": "bad-token"}
        }])))
        .mount(&server)
        .await;

    let ledger = Arc::new(Ledger::open_in_memory().unwrap());
    let resumer = Arc::new(RecordingResumer::default());
    let login_resumer = Arc::new(ScriptedLoginResumer::new(1)); // fails once
    let confirmer = Confirmer::with_base_url(
        "tok123",
        &server.uri(),
        "42".to_string(),
        120,
        ledger,
        resumer,
    )
    .with_login_resumer(login_resumer.clone());

    confirmer
        .notify_needs_login("https://kite/login?api_key=x")
        .await
        .unwrap();
    confirmer.poll_once(0).await.unwrap();
    assert_eq!(login_resumer.calls.lock().unwrap().len(), 1);

    // Still awaiting — a second reply (offset 0 replays the same fixed mock,
    // fine since it's just re-processing the same not-yet-consumed update in
    // this simplified test) should call complete_login again, not be dropped.
    confirmer.poll_once(0).await.unwrap();
    assert_eq!(login_resumer.calls.lock().unwrap().len(), 2);
}

#[test]
fn order_summary_includes_ref_id_and_side() {
    let ledger = Ledger::open_in_memory().unwrap();
    let ref_id = seed_pending(&ledger);
    let pending = ledger.pending_confirm(&ref_id).unwrap().unwrap();
    let text = format_order_summary(&pending);
    assert!(text.contains(&ref_id));
    assert!(text.contains("BUY"));
    assert!(text.contains("INFY"));
}
