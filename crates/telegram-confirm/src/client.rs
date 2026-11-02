//! A thin `reqwest` wrapper over the Telegram Bot API — mirrors
//! `kite-client`'s own "hand-rolled REST client, no business logic" pattern
//! rather than pulling in a full bot framework.

use serde::Deserialize;

use crate::ConfirmError;

pub struct TelegramClient {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(bound = "T: serde::de::DeserializeOwned")]
struct Envelope<T> {
    ok: bool,
    #[serde(default)]
    result: Option<T>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    pub update_id: i64,
    #[serde(default)]
    pub callback_query: Option<CallbackQuery>,
    #[serde(default)]
    pub message: Option<Message>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: TgUser,
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TgUser {
    pub id: i64,
}

/// A plain text message — used for the login-token reply flow, distinct from
/// the button-callback flow order confirmations use.
#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub chat: Chat,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub id: i64,
}

impl TelegramClient {
    /// `base_url` defaults to `https://api.telegram.org`; overridable for tests.
    pub fn new(bot_token: &str) -> Self {
        Self::with_base_url(bot_token, "https://api.telegram.org")
    }

    pub fn with_base_url(bot_token: &str, base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: format!("{base_url}/bot{bot_token}"),
        }
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &[(&str, String)],
    ) -> Result<T, ConfirmError> {
        let url = format!("{}/{method}", self.base_url);
        let resp = self
            .http
            .post(url)
            .form(params)
            .send()
            .await
            .map_err(|e| ConfirmError::Network(e.to_string()))?;
        let env: Envelope<T> = resp
            .json()
            .await
            .map_err(|e| ConfirmError::Network(format!("unparseable response: {e}")))?;
        if !env.ok {
            return Err(ConfirmError::Api(
                env.description
                    .unwrap_or_else(|| "unknown error".to_string()),
            ));
        }
        env.result
            .ok_or_else(|| ConfirmError::Api("empty result".to_string()))
    }

    /// Startup health check (DESIGN.md §2.7) — must succeed before the server
    /// starts in any mode that can reach `NeedsHumanConfirm`.
    pub async fn get_me(&self) -> Result<(), ConfirmError> {
        let _: serde_json::Value = self.call("getMe", &[]).await?;
        Ok(())
    }

    /// Sends the order summary with Approve/Reject inline buttons, carrying
    /// `nonce` in `callback_data` as `"<nonce>:approve"` / `"<nonce>:reject"`.
    pub async fn send_message_with_buttons(
        &self,
        chat_id: &str,
        text: &str,
        nonce: &str,
    ) -> Result<i64, ConfirmError> {
        let keyboard = serde_json::json!({
            "inline_keyboard": [[
                {"text": "Approve", "callback_data": format!("{nonce}:approve")},
                {"text": "Reject", "callback_data": format!("{nonce}:reject")},
            ]]
        });
        #[derive(Deserialize)]
        struct Sent {
            message_id: i64,
        }
        let sent: Sent = self
            .call(
                "sendMessage",
                &[
                    ("chat_id", chat_id.to_string()),
                    ("text", text.to_string()),
                    ("reply_markup", keyboard.to_string()),
                ],
            )
            .await?;
        Ok(sent.message_id)
    }

    /// Plain text message, no buttons — used for login prompts/status, not
    /// order confirmations.
    pub async fn send_message(&self, chat_id: &str, text: &str) -> Result<(), ConfirmError> {
        let _: serde_json::Value = self
            .call(
                "sendMessage",
                &[("chat_id", chat_id.to_string()), ("text", text.to_string())],
            )
            .await?;
        Ok(())
    }

    /// Long-poll for new updates. `timeout_secs` is Telegram's own long-poll
    /// timeout, not a client-side one.
    pub async fn get_updates(
        &self,
        offset: i64,
        timeout_secs: u64,
    ) -> Result<Vec<Update>, ConfirmError> {
        self.call(
            "getUpdates",
            &[
                ("offset", offset.to_string()),
                ("timeout", timeout_secs.to_string()),
            ],
        )
        .await
    }

    pub async fn answer_callback_query(&self, callback_query_id: &str) -> Result<(), ConfirmError> {
        let _: serde_json::Value = self
            .call(
                "answerCallbackQuery",
                &[("callback_query_id", callback_query_id.to_string())],
            )
            .await?;
        Ok(())
    }
}
