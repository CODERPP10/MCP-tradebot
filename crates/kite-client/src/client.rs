//! The Kite Connect REST client (DESIGN.md §2.1). Pure transport: auth headers,
//! the session checksum, a client-side rate limiter, typed (de)serialisation,
//! and the [`KiteError`] taxonomy. **No business logic, no retries.**
//!
//! This crate never logs secrets, tokens, or request/response bodies — it does
//! no logging at all; the engine decides what is safe to record.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use url::Url;

use domain::Variety;

use crate::error::{ErrorBody, KiteError};
use crate::models::{
    wire_variety, Envelope, Holding, Instrument, LtpQuote, Order, OrderId, OrderIdData,
    PlaceOrderRequest, PositionsData, Profile, Quote, SegmentMargins, SessionData,
};
use crate::rate_limit::RateLimiter;

/// Kite Connect API root.
pub const DEFAULT_BASE_URL: &str = "https://api.kite.trade";

/// Construction parameters for [`KiteClient`].
#[derive(Debug, Clone)]
pub struct KiteConfig {
    /// API root. Override in tests to point at a mock server.
    pub base_url: String,
    /// The public API key (identifier, not a secret).
    pub api_key: String,
    /// Per-request timeout (connect + read).
    pub timeout: Duration,
}

impl KiteConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: api_key.into(),
            timeout: Duration::from_secs(10),
        }
    }
}

/// Thread-safe, cheap to `clone` (shares one connection pool + rate limiter).
#[derive(Clone)]
pub struct KiteClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    base_url: Url,
    api_key: String,
    access_token: RwLock<Option<String>>,
    limiter: RateLimiter,
}

impl KiteClient {
    /// Build a client with the conservative default rate limiter (2 req/s).
    pub fn new(config: KiteConfig) -> Result<Self, KiteError> {
        Self::with_rate_limiter(config, RateLimiter::conservative())
    }

    pub fn with_rate_limiter(config: KiteConfig, limiter: RateLimiter) -> Result<Self, KiteError> {
        let base_url = Url::parse(&config.base_url)
            .map_err(|e| KiteError::Unknown(format!("invalid base_url: {e}")))?;
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .user_agent("tradebot-kite-client/0.1")
            .build()
            .map_err(KiteError::from)?;
        Ok(Self {
            inner: Arc::new(Inner {
                http,
                base_url,
                api_key: config.api_key,
                access_token: RwLock::new(None),
                limiter,
            }),
        })
    }

    /// Install an access token obtained elsewhere (e.g. unsealed by the session
    /// layer from the secret store). Subsequent authenticated calls use it.
    pub async fn set_access_token(&self, token: impl Into<String>) {
        *self.inner.access_token.write().await = Some(token.into());
    }

    pub async fn has_access_token(&self) -> bool {
        self.inner.access_token.read().await.is_some()
    }

    /// `SHA256(api_key + request_token + api_secret)` — Kite's login checksum.
    fn checksum(api_key: &str, request_token: &str, api_secret: &str) -> String {
        let mut h = Sha256::new();
        h.update(api_key.as_bytes());
        h.update(request_token.as_bytes());
        h.update(api_secret.as_bytes());
        hex::encode(h.finalize())
    }

    // ---- session -----------------------------------------------------------

    /// `POST /session/token`. On success the returned `access_token` is stored
    /// on this client for subsequent calls; the caller still seals it.
    pub async fn generate_session(
        &self,
        request_token: &str,
        api_secret: &str,
    ) -> Result<SessionData, KiteError> {
        let checksum = Self::checksum(&self.inner.api_key, request_token, api_secret);
        let form = [
            ("api_key", self.inner.api_key.as_str()),
            ("request_token", request_token),
            ("checksum", checksum.as_str()),
        ];
        let data: SessionData = self
            .send(
                self.request(reqwest::Method::POST, "/session/token")
                    .await?
                    .form(&form),
            )
            .await?;
        self.set_access_token(data.access_token.clone()).await;
        Ok(data)
    }

    // ---- orders (mutating) -----------------------------------------------------

    /// `POST /orders/:variety`. Returns the broker order id. **Not retried by
    /// anyone** — a timeout here is "state unknown", handled by reconciliation.
    pub async fn place_order(&self, req: &PlaceOrderRequest) -> Result<OrderId, KiteError> {
        let path = format!("/orders/{}", wire_variety(req.variety));
        let params = req.form_params();
        let data: OrderIdData = self
            .send(
                self.request(reqwest::Method::POST, &path)
                    .await?
                    .form(&params),
            )
            .await?;
        Ok(OrderId(data.order_id))
    }

    /// `DELETE /orders/:variety/:order_id`.
    pub async fn cancel_order(
        &self,
        variety: Variety,
        order_id: &str,
    ) -> Result<OrderId, KiteError> {
        let path = format!("/orders/{}/{}", wire_variety(variety), order_id);
        let data: OrderIdData = self
            .send(self.request(reqwest::Method::DELETE, &path).await?)
            .await?;
        Ok(OrderId(data.order_id))
    }

    // ---- orders (read) ------------------------------------------------------

    /// `GET /orders` — today's order book.
    pub async fn orders(&self) -> Result<Vec<Order>, KiteError> {
        self.send(self.request(reqwest::Method::GET, "/orders").await?)
            .await
    }

    /// `GET /orders/:order_id` — the status transitions of one order.
    pub async fn order_history(&self, order_id: &str) -> Result<Vec<Order>, KiteError> {
        let path = format!("/orders/{order_id}");
        self.send(self.request(reqwest::Method::GET, &path).await?)
            .await
    }

    // ---- portfolio --------------------------------------------------------

    /// `GET /portfolio/positions`.
    pub async fn positions(&self) -> Result<PositionsData, KiteError> {
        self.send(
            self.request(reqwest::Method::GET, "/portfolio/positions")
                .await?,
        )
        .await
    }

    /// `GET /portfolio/holdings`.
    pub async fn holdings(&self) -> Result<Vec<Holding>, KiteError> {
        self.send(
            self.request(reqwest::Method::GET, "/portfolio/holdings")
                .await?,
        )
        .await
    }

    /// `GET /user/margins/equity` — v1 is equity-only.
    pub async fn margins_equity(&self) -> Result<SegmentMargins, KiteError> {
        self.send(
            self.request(reqwest::Method::GET, "/user/margins/equity")
                .await?,
        )
        .await
    }

    /// `GET /user/profile`.
    pub async fn profile(&self) -> Result<Profile, KiteError> {
        self.send(self.request(reqwest::Method::GET, "/user/profile").await?)
            .await
    }

    // ---- market data ------------------------------------------------------

    /// `GET /quote?i=NSE:INFY&i=…` — full quotes keyed by `"EXCH:SYMBOL"`.
    pub async fn quote(&self, instruments: &[&str]) -> Result<HashMap<String, Quote>, KiteError> {
        self.quote_like("/quote", instruments).await
    }

    /// `GET /quote/ltp?i=…` — last price only.
    pub async fn ltp(&self, instruments: &[&str]) -> Result<HashMap<String, LtpQuote>, KiteError> {
        self.quote_like("/quote/ltp", instruments).await
    }

    async fn quote_like<T: DeserializeOwned>(
        &self,
        path: &str,
        instruments: &[&str],
    ) -> Result<HashMap<String, T>, KiteError> {
        let query: Vec<(&str, &str)> = instruments.iter().map(|i| ("i", *i)).collect();
        self.send(
            self.request(reqwest::Method::GET, path)
                .await?
                .query(&query),
        )
        .await
    }

    // ---- instruments (CSV, not the JSON envelope) ------------------------

    /// `GET /instruments` (all) or `GET /instruments/:exchange`. The response is
    /// a CSV dump, not the JSON envelope, so this path is handled specially.
    pub async fn instruments(&self, exchange: Option<&str>) -> Result<Vec<Instrument>, KiteError> {
        let path = match exchange {
            Some(e) => format!("/instruments/{e}"),
            None => "/instruments".to_string(),
        };
        self.inner.limiter.acquire().await;
        let resp = self
            .request(reqwest::Method::GET, &path)
            .await?
            .send()
            .await
            .map_err(KiteError::from)?;
        let status = resp.status();
        let body = resp.text().await.map_err(KiteError::from)?;
        if !status.is_success() {
            return Err(classify_error(status.as_u16(), &body));
        }
        Instrument::parse_csv(&body).map_err(|e| KiteError::Unknown(e.to_string()))
    }

    // ---- internals ------------------------------------------------------

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, KiteError> {
        let url = self
            .inner
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|e| KiteError::Unknown(format!("bad path {path}: {e}")))?;
        let mut rb = self
            .inner
            .http
            .request(method, url)
            .header("X-Kite-Version", "3");
        if let Some(token) = self.inner.access_token.read().await.as_ref() {
            rb = rb.header(
                reqwest::header::AUTHORIZATION,
                format!("token {}:{}", self.inner.api_key, token),
            );
        }
        Ok(rb)
    }

    async fn send<T: DeserializeOwned>(&self, rb: reqwest::RequestBuilder) -> Result<T, KiteError> {
        self.inner.limiter.acquire().await;
        let resp = rb.send().await.map_err(KiteError::from)?;
        let status = resp.status();
        let body = resp.text().await.map_err(KiteError::from)?;

        if status.is_success() {
            let env: Envelope<T> = serde_json::from_str(&body)
                .map_err(|e| KiteError::Unknown(format!("unexpected success body: {e}")))?;
            Ok(env.data)
        } else {
            Err(classify_error(status.as_u16(), &body))
        }
    }
}

/// Parse Kite's error envelope out of `body` and map it; fall back to the raw
/// HTTP status if the body is not the expected shape.
fn classify_error(status: u16, body: &str) -> KiteError {
    match serde_json::from_str::<ErrorBody>(body) {
        Ok(err) => KiteError::from_envelope(status, err.error_type.as_deref(), &err.message),
        Err(_) => KiteError::from_envelope(status, None, "unparseable error body"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_matches_known_sha256() {
        // SHA256("key" + "reqtoken" + "secret")
        assert_eq!(
            KiteClient::checksum("key", "reqtoken", "secret"),
            "6d19fc70c83d67f6f65d2596fb4ffb8fad5215a84cafcaf4dbb67e9c7f516e66"
        );
    }
}
