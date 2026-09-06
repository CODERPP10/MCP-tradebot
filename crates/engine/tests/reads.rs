//! Read-service tests: a fake `TokenProvider` + a `wiremock` Kite.

use std::sync::Arc;

use async_trait::async_trait;
use engine::{Engine, EngineError};
use kite_client::{KiteClient, KiteConfig};
use session::{NeedsLogin, SessionError, TokenProvider, TokenStatus};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct FakeTokens(TokenStatus);

#[async_trait]
impl TokenProvider for FakeTokens {
    async fn access_token(&self) -> Result<TokenStatus, SessionError> {
        Ok(self.0.clone())
    }
}

fn engine_for(server: &MockServer, tokens: TokenStatus) -> Engine {
    let mut cfg = KiteConfig::new("api_key_123");
    cfg.base_url = server.uri();
    Engine::new(KiteClient::new(cfg).unwrap(), Arc::new(FakeTokens(tokens)))
}

fn ok_json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_owned().into_bytes(), "application/json")
}

#[tokio::test]
async fn needs_login_short_circuits_before_any_kite_call() {
    let server = MockServer::start().await;
    // no mocks mounted — a call would 404 and fail the test differently
    let eng = engine_for(
        &server,
        TokenStatus::NeedsLogin(NeedsLogin {
            login_url: "https://kite/login?api_key=x".into(),
        }),
    );
    match eng.margins().await {
        Err(EngineError::NeedsLogin { login_url }) => assert!(login_url.contains("api_key=x")),
        other => panic!("expected NeedsLogin, got {other:?}"),
    }
}

#[tokio::test]
async fn margins_are_shaped_to_the_view() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/margins/equity"))
        .respond_with(ok_json(
            r#"{"status":"success","data":{"enabled":true,"net":15000.5,
                "available":{"live_balance":12000.25,"cash":12000.25,"opening_balance":0,"intraday_payin":0},
                "utilised":{"debits":3000.25,"exposure":0,"m2m_realised":0,"m2m_unrealised":0}}}"#,
        ))
        .mount(&server)
        .await;

    let eng = engine_for(&server, TokenStatus::Valid("tok".into()));
    let m = eng.margins().await.unwrap();
    assert_eq!(m.available, 12000.25);
    assert_eq!(m.utilised, 3000.25);
    assert_eq!(m.net, 15000.5);
}

#[tokio::test]
async fn quote_prefixes_exchange_and_unwraps_the_map() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(ok_json(
            r#"{"status":"success","data":{"NSE:INFY":{"instrument_token":408065,"last_price":1500.0,
                "lower_circuit_limit":1000.0,"upper_circuit_limit":2000.0,
                "ohlc":{"open":1490,"high":1510,"low":1480,"close":1495},"volume":100}}}"#,
        ))
        .mount(&server)
        .await;

    let eng = engine_for(&server, TokenStatus::Valid("tok".into()));
    let q = eng.quote("infy").await.unwrap();
    assert_eq!(q.last_price, 1500.0);
    assert!(q.tradable);
    assert!(!q.at_circuit_limit);
    assert_eq!(q.ohlc.close, 1495.0);
}

#[tokio::test]
async fn list_orders_extracts_ref_id_from_tag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orders"))
        .respond_with(ok_json(
            r#"{"status":"success","data":[
                {"order_id":"25091","tradingsymbol":"INFY","transaction_type":"BUY","quantity":1,
                 "filled_quantity":1,"price":1500.0,"status":"COMPLETE","tag":"tradebot:abc123"},
                {"order_id":"25092","tradingsymbol":"TCS","transaction_type":"SELL","quantity":2,
                 "filled_quantity":0,"price":3900.0,"status":"OPEN"}]}"#,
        ))
        .mount(&server)
        .await;

    let eng = engine_for(&server, TokenStatus::Valid("tok".into()));
    let v = eng.orders().await.unwrap();
    assert_eq!(v.orders[0].ref_id.as_deref(), Some("abc123"));
    assert_eq!(v.orders[1].ref_id, None);
}
