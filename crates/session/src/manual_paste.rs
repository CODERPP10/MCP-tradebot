//! [`ManualPaste`] — the v1 [`TokenProvider`].

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Utc};
use kite_client::KiteClient;
use secret_store::{SecretError, SharedSecretStore, KITE_ACCESS_TOKEN, KITE_API_SECRET};
use serde::{Deserialize, Serialize};

use crate::{
    NeedsLogin, SessionError, TokenProvider, TokenStatus, IST_OFFSET_SECS, KITE_LOGIN_URL,
};

/// What `ManualPaste` seals under [`KITE_ACCESS_TOKEN`]: the token plus its
/// issue time and the local best-effort estimate of when Kite will reject it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedToken {
    pub access_token: String,
    pub issued_at: DateTime<FixedOffset>,
    pub assumed_invalid_after: DateTime<FixedOffset>,
}

/// Drives an interactive login and caches the sealed token. `Clone` is cheap
/// (shares the store handle and the Kite client).
#[derive(Clone)]
pub struct ManualPaste {
    store: SharedSecretStore,
    kite: KiteClient,
    api_key: String,
    /// Injectable clock — real code uses [`now_ist`].
    now: fn() -> DateTime<FixedOffset>,
}

impl ManualPaste {
    pub fn new(store: SharedSecretStore, kite: KiteClient, api_key: impl Into<String>) -> Self {
        Self {
            store,
            kite,
            api_key: api_key.into(),
            now: now_ist,
        }
    }

    #[cfg(test)]
    fn with_clock(mut self, now: fn() -> DateTime<FixedOffset>) -> Self {
        self.now = now;
        self
    }

    /// The Kite Connect login URL for this API key. Open it, log in, and copy
    /// the `request_token` query parameter from the redirect.
    pub fn login_url(&self) -> String {
        format!("{KITE_LOGIN_URL}?api_key={}&v=3", self.api_key)
    }

    /// Exchange a pasted `request_token` for an access token and seal it.
    /// Overwrites any previously sealed token.
    pub async fn complete_login(&self, request_token: &str) -> Result<SealedToken, SessionError> {
        let api_secret = self.get_secret(KITE_API_SECRET)?;
        let session = self
            .kite
            .generate_session(request_token.trim(), &api_secret)
            .await
            .map_err(|e| SessionError::Login(e.to_string()))?;

        let issued_at = (self.now)();
        let sealed = SealedToken {
            access_token: session.access_token,
            issued_at,
            assumed_invalid_after: next_token_expiry(issued_at),
        };
        let json =
            serde_json::to_string(&sealed).map_err(|e| SessionError::Corrupt(e.to_string()))?;
        self.store
            .lock()
            .expect("secret store mutex poisoned")
            .put(KITE_ACCESS_TOKEN, &json)
            .map_err(|e| SessionError::Secret(e.to_string()))?;
        Ok(sealed)
    }

    /// Read the sealed token record, if any.
    pub fn sealed_token(&self) -> Result<Option<SealedToken>, SessionError> {
        match self
            .store
            .lock()
            .expect("secret store mutex poisoned")
            .get(KITE_ACCESS_TOKEN)
        {
            Ok(raw) => serde_json::from_str(&raw)
                .map(Some)
                .map_err(|e| SessionError::Corrupt(e.to_string())),
            Err(SecretError::NotFound(_)) => Ok(None),
            Err(e) => Err(SessionError::Secret(e.to_string())),
        }
    }

    fn get_secret(&self, key: &str) -> Result<String, SessionError> {
        self.store
            .lock()
            .expect("secret store mutex poisoned")
            .get(key)
            .map_err(|e| SessionError::Secret(e.to_string()))
    }

    fn needs_login(&self) -> TokenStatus {
        TokenStatus::NeedsLogin(NeedsLogin {
            login_url: self.login_url(),
        })
    }
}

#[async_trait]
impl TokenProvider for ManualPaste {
    async fn access_token(&self) -> Result<TokenStatus, SessionError> {
        match self.sealed_token()? {
            Some(t) if (self.now)() < t.assumed_invalid_after => {
                Ok(TokenStatus::Valid(t.access_token))
            }
            _ => Ok(self.needs_login()),
        }
    }
}

/// Current time in IST (fixed +05:30).
pub fn now_ist() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&ist())
}

fn ist() -> FixedOffset {
    FixedOffset::east_opt(IST_OFFSET_SECS).expect("valid IST offset")
}

/// The next 06:00 IST strictly after `issued_at` — Kite invalidates tokens at
/// the start of each trading day. A best-effort local estimate only.
fn next_token_expiry(issued_at: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    let ist = ist();
    let six_am_today = ist
        .with_ymd_and_hms(
            issued_at.year(),
            issued_at.month(),
            issued_at.day(),
            6,
            0,
            0,
        )
        .single()
        .expect("06:00 IST is unambiguous");
    if issued_at < six_am_today {
        six_am_today
    } else {
        six_am_today + Duration::days(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secret_store::{shared, Argon2Params, SealedFileStore, SecretStore};

    fn ist_dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<FixedOffset> {
        ist().with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn test_store() -> (tempfile::TempDir, SharedSecretStore) {
        let dir = tempfile::tempdir().unwrap();
        let mut s = SealedFileStore::create(
            dir.path().join("s.json"),
            "passphrase123",
            Argon2Params {
                m_cost_kib: 16,
                t_cost: 1,
                p_cost: 1,
            },
        )
        .unwrap();
        s.put(KITE_API_SECRET, "the-secret").unwrap();
        (dir, shared(s))
    }

    fn kite() -> KiteClient {
        KiteClient::new(kite_client::KiteConfig::new("api_key_123")).unwrap()
    }

    #[test]
    fn expiry_is_next_0600_ist() {
        // issued before 6am → expires 6am same day
        assert_eq!(
            next_token_expiry(ist_dt(2026, 9, 7, 5, 30)),
            ist_dt(2026, 9, 7, 6, 0)
        );
        // issued after 6am → expires 6am next day
        assert_eq!(
            next_token_expiry(ist_dt(2026, 9, 7, 9, 15)),
            ist_dt(2026, 9, 8, 6, 0)
        );
    }

    #[test]
    fn login_url_carries_api_key() {
        let (_d, store) = test_store();
        let mp = ManualPaste::new(store, kite(), "api_key_123");
        assert_eq!(
            mp.login_url(),
            "https://kite.zerodha.com/connect/login?api_key=api_key_123&v=3"
        );
    }

    #[tokio::test]
    async fn no_sealed_token_means_needs_login() {
        let (_d, store) = test_store();
        let mp = ManualPaste::new(store, kite(), "api_key_123");
        match mp.access_token().await.unwrap() {
            TokenStatus::NeedsLogin(n) => assert!(n.login_url.contains("api_key_123")),
            other => panic!("expected NeedsLogin, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn valid_token_is_returned_then_expires() {
        let (_d, store) = test_store();
        // Seal a token by hand (as complete_login would).
        let sealed = SealedToken {
            access_token: "tok-abc".into(),
            issued_at: ist_dt(2026, 9, 7, 9, 0),
            assumed_invalid_after: ist_dt(2026, 9, 8, 6, 0),
        };
        store
            .lock()
            .unwrap()
            .put(KITE_ACCESS_TOKEN, &serde_json::to_string(&sealed).unwrap())
            .unwrap();

        let before = ManualPaste::new(store.clone(), kite(), "k")
            .with_clock(|| ist_dt_static(2026, 9, 7, 15, 0));
        assert_eq!(
            before.access_token().await.unwrap(),
            TokenStatus::Valid("tok-abc".into())
        );

        let after =
            ManualPaste::new(store, kite(), "k").with_clock(|| ist_dt_static(2026, 9, 8, 7, 0));
        assert!(matches!(
            after.access_token().await.unwrap(),
            TokenStatus::NeedsLogin(_)
        ));
    }

    // fn-pointer clocks can't capture, so use fixed helpers.
    fn ist_dt_static(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(IST_OFFSET_SECS)
            .unwrap()
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .unwrap()
    }

    #[tokio::test]
    async fn complete_login_exchanges_request_token_and_seals_it() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/session/token"))
            .and(body_string_contains("request_token=req-xyz"))
            .and(body_string_contains("api_key=api_key_123"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                br#"{"status":"success","data":{"user_id":"AB1","user_name":"T","access_token":"live-tok-1","public_token":"p","login_time":"2026-09-07 08:45:00"}}"#.to_vec(),
                "application/json",
            ))
            .mount(&server)
            .await;

        let (_d, store) = test_store();
        let mut cfg = kite_client::KiteConfig::new("api_key_123");
        cfg.base_url = server.uri();
        let mp = ManualPaste::new(store.clone(), KiteClient::new(cfg).unwrap(), "api_key_123");

        let sealed = mp.complete_login(" req-xyz ").await.unwrap();
        assert_eq!(sealed.access_token, "live-tok-1");
        assert!(sealed.assumed_invalid_after > sealed.issued_at);

        // token is persisted and unseals to the same value
        let reread = mp.sealed_token().unwrap().unwrap();
        assert_eq!(reread.access_token, "live-tok-1");
    }
}
