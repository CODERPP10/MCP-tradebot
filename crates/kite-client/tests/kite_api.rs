//! Fixture-based tests for `kite-client` (DESIGN.md §2.1: "Tested against
//! recorded HTTP fixtures (no live calls in CI)"). Every response body under
//! `tests/fixtures/` is a trimmed copy of a real Kite Connect payload.

use std::time::Duration;

use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use domain::{Exchange, OrderType, Product, Side, Validity, Variety};
use kite_client::{KiteClient, KiteConfig, KiteError, PlaceOrderRequest};

const F_SESSION: &str = include_str!("fixtures/session_token.json");
const F_PLACE_OK: &str = include_str!("fixtures/place_order_success.json");
const F_ERR_TOKEN: &str = include_str!("fixtures/error_token.json");
const F_ERR_INPUT: &str = include_str!("fixtures/error_input.json");
const F_ORDERS: &str = include_str!("fixtures/orders.json");
const F_POSITIONS: &str = include_str!("fixtures/positions.json");
const F_HOLDINGS: &str = include_str!("fixtures/holdings.json");
const F_MARGINS: &str = include_str!("fixtures/margins_equity.json");
const F_LTP: &str = include_str!("fixtures/ltp.json");
const F_QUOTE: &str = include_str!("fixtures/quote.json");
const F_INSTRUMENTS: &str = include_str!("fixtures/instruments_nse.csv");

fn json(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

async fn client(server: &MockServer, timeout: Duration) -> KiteClient {
    let mut cfg = KiteConfig::new("test_api_key");
    cfg.base_url = server.uri();
    cfg.timeout = timeout;
    let c = KiteClient::new(cfg).unwrap();
    c.set_access_token("test_access_token").await;
    c
}

fn sample_order() -> PlaceOrderRequest {
    PlaceOrderRequest {
        tradingsymbol: "INFY".into(),
        exchange: Exchange::Nse,
        transaction_type: Side::Buy,
        order_type: OrderType::Limit,
        quantity: 3,
        product: Product::Cnc,
        validity: Validity::Day,
        variety: Variety::Regular,
        price: Some(1500.0),
        tag: Some("tradebot:01J9ABC".into()),
    }
}

#[tokio::test]
async fn generate_session_sends_checksum_and_stores_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/session/token"))
        .and(body_string_contains("api_key=test_api_key"))
        .and(body_string_contains("request_token=reqtok123"))
        .and(body_string_contains("checksum="))
        .respond_with(json(200, F_SESSION))
        .mount(&server)
        .await;

    let mut cfg = KiteConfig::new("test_api_key");
    cfg.base_url = server.uri();
    let c = KiteClient::new(cfg).unwrap();
    assert!(!c.has_access_token().await);

    let session = c
        .generate_session("reqtok123", "the_api_secret")
        .await
        .unwrap();
    assert_eq!(session.access_token, "fixture_access_token_abc123");
    assert_eq!(session.user_id, "AB1234");
    assert!(c.has_access_token().await);
}

#[tokio::test]
async fn place_order_hits_variety_path_with_auth_header_and_returns_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orders/regular"))
        .and(header(
            "authorization",
            "token test_api_key:test_access_token",
        ))
        .and(header("x-kite-version", "3"))
        .and(body_string_contains("transaction_type=BUY"))
        .and(body_string_contains("order_type=LIMIT"))
        .and(body_string_contains("product=CNC"))
        .and(body_string_contains("price=1500.00"))
        .respond_with(json(200, F_PLACE_OK))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    let id = c.place_order(&sample_order()).await.unwrap();
    assert_eq!(id.0, "250906000000001");
}

#[tokio::test]
async fn token_exception_maps_to_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orders/regular"))
        .respond_with(json(403, F_ERR_TOKEN))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    assert!(matches!(
        c.place_order(&sample_order()).await,
        Err(KiteError::Auth)
    ));
}

#[tokio::test]
async fn input_exception_maps_to_rejected_with_code_and_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orders/regular"))
        .respond_with(json(400, F_ERR_INPUT))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    match c.place_order(&sample_order()).await {
        Err(KiteError::Rejected { code, message }) => {
            assert_eq!(code, "InputException");
            assert!(message.contains("NOSUCH"));
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test]
async fn http_429_maps_to_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orders"))
        .respond_with(json(
            429,
            r#"{"status":"error","message":"Too many requests","error_type":"TooManyRequestsException"}"#,
        ))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    assert!(matches!(c.orders().await, Err(KiteError::RateLimited)));
}

#[tokio::test]
async fn slow_response_maps_to_network_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/profile"))
        .respond_with(json(200, "{}").set_delay(Duration::from_millis(400)))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_millis(60)).await;
    assert!(matches!(c.profile().await, Err(KiteError::NetworkTimeout)));
}

#[tokio::test]
async fn orders_parse_book_and_tags() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orders"))
        .respond_with(json(200, F_ORDERS))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    let orders = c.orders().await.unwrap();
    assert_eq!(orders.len(), 2);
    assert_eq!(orders[0].status, "COMPLETE");
    assert_eq!(orders[0].filled_quantity, 3);
    assert_eq!(orders[0].tag.as_deref(), Some("tradebot:01J9ABC"));
    assert_eq!(orders[1].status, "OPEN");
    assert_eq!(orders[1].pending_quantity, 1);
}

#[tokio::test]
async fn positions_parse_net_and_day() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/portfolio/positions"))
        .respond_with(json(200, F_POSITIONS))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    let p = c.positions().await.unwrap();
    assert_eq!(p.net.len(), 1);
    assert_eq!(p.net[0].tradingsymbol, "INFY");
    assert_eq!(p.net[0].quantity, 3);
    assert_eq!(p.day.len(), 1);
}

#[tokio::test]
async fn holdings_parse() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/portfolio/holdings"))
        .respond_with(json(200, F_HOLDINGS))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    let h = c.holdings().await.unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].quantity, 10);
}

#[tokio::test]
async fn margins_equity_parse() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/margins/equity"))
        .respond_with(json(200, F_MARGINS))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    let m = c.margins_equity().await.unwrap();
    assert!(m.enabled);
    assert_eq!(m.available.live_balance, 92750.35);
    assert_eq!(m.utilised.debits, 7249.65);
}

#[tokio::test]
async fn ltp_parse_keyed_map() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote/ltp"))
        .and(query_param("i", "NSE:INFY"))
        .respond_with(json(200, F_LTP))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    let ltp = c.ltp(&["NSE:INFY", "NSE:TCS"]).await.unwrap();
    assert_eq!(ltp["NSE:INFY"].last_price, 1512.4);
    assert_eq!(ltp["NSE:TCS"].instrument_token, 2953217);
}

#[tokio::test]
async fn quote_parse_circuit_limits_and_ohlc() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/quote"))
        .respond_with(json(200, F_QUOTE))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    let q = c.quote(&["NSE:INFY"]).await.unwrap();
    let infy = &q["NSE:INFY"];
    assert_eq!(infy.last_price, 1512.4);
    assert_eq!(infy.lower_circuit_limit, 1361.2);
    assert_eq!(infy.ohlc.close, 1499.0);
}

#[tokio::test]
async fn instruments_parse_csv_over_http() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/instruments/NSE"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(F_INSTRUMENTS.as_bytes().to_vec(), "text/csv"),
        )
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    let inst = c.instruments(Some("NSE")).await.unwrap();
    assert_eq!(inst.len(), 3);
    assert_eq!(inst[0].tradingsymbol, "INFY");
    assert_eq!(inst[0].instrument_token, 408065);
    assert!(inst.iter().all(|i| i.instrument_type == "EQ"));
}

#[tokio::test]
async fn cancel_order_uses_delete_and_variety_path() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/orders/regular/250906000000002"))
        .respond_with(json(
            200,
            r#"{"status":"success","data":{"order_id":"250906000000002"}}"#,
        ))
        .mount(&server)
        .await;

    let c = client(&server, Duration::from_secs(5)).await;
    let id = c
        .cancel_order(Variety::Regular, "250906000000002")
        .await
        .unwrap();
    assert_eq!(id.0, "250906000000002");
}
