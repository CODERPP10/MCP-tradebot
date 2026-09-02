//! Sealed at-rest storage for all secrets (DESIGN.md §2.2).
//!
//! Stub — implemented in `feat/secret-store`. v1 impl `SealedFileStore`:
//! per-record XChaCha20-Poly1305, key derived from an operator passphrase via
//! Argon2id, key material zeroized on drop and never written to disk.

/// Well-known record keys held by the store.
pub const KITE_API_KEY: &str = "kite_api_key";
pub const KITE_API_SECRET: &str = "kite_api_secret";
pub const KITE_ACCESS_TOKEN: &str = "kite_access_token";
pub const TELEGRAM_BOT_TOKEN: &str = "telegram_bot_token";
pub const TELEGRAM_CHAT_ID: &str = "telegram_chat_id";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret store is not implemented yet (feat/secret-store)")]
    NotImplemented,
    #[error("no such secret: {0}")]
    NotFound(String),
    #[error("wrong passphrase or corrupt store")]
    Unsealing,
}

/// Sealed-store interface. Concrete impls: `SealedFileStore` (v1), `KmsStore` (later).
pub trait SecretStore {
    fn get(&self, key: &str) -> Result<String, SecretError>;
    fn put(&self, key: &str, value: &str) -> Result<(), SecretError>;
    fn list_keys(&self) -> Result<Vec<String>, SecretError>;
}
