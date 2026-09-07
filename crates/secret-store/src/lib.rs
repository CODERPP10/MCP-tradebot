//! Sealed at-rest storage for all secrets (DESIGN.md §2.2).
//!
//! v1 impl [`SealedFileStore`]: one file, each record sealed with
//! XChaCha20-Poly1305 under a key derived from the operator passphrase via
//! Argon2id. The derived key lives in memory only (zeroized on drop) and is
//! never written to disk. The passphrase is required on **every** process start
//! (server and CLI) — a crash does not auto-recover unattended.

mod sealed_file;

use std::sync::{Arc, Mutex};

pub use sealed_file::{Argon2Params, SealedFileStore};

/// A [`SecretStore`] shared across the engine, session layer, and CLI. `put`
/// needs `&mut self`, so the shared handle is a `Mutex`.
pub type SharedSecretStore = Arc<Mutex<dyn SecretStore + Send>>;

/// Wrap a concrete store in a [`SharedSecretStore`].
pub fn shared(store: impl SecretStore + Send + 'static) -> SharedSecretStore {
    Arc::new(Mutex::new(store))
}

/// Well-known record keys held by the store.
pub const KITE_API_KEY: &str = "kite_api_key";
pub const KITE_API_SECRET: &str = "kite_api_secret";
pub const KITE_ACCESS_TOKEN: &str = "kite_access_token";
pub const TELEGRAM_BOT_TOKEN: &str = "telegram_bot_token";
pub const TELEGRAM_CHAT_ID: &str = "telegram_chat_id";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("no such secret: {0}")]
    NotFound(String),
    /// Wrong passphrase, tampered ciphertext, or a corrupt/foreign store file.
    #[error("wrong passphrase or corrupt store")]
    Unsealing,
    #[error("secret store I/O error: {0}")]
    Io(String),
    #[error("secret store file is malformed: {0}")]
    Corrupt(String),
    /// A record key or value that cannot be represented on disk.
    #[error("invalid secret {0}")]
    Invalid(String),
}

/// Sealed-store interface. Concrete impls: [`SealedFileStore`] (v1),
/// `KmsStore` (later, slots in behind this trait).
pub trait SecretStore {
    /// Decrypt and return the record, or [`SecretError::NotFound`].
    fn get(&self, key: &str) -> Result<String, SecretError>;
    /// Seal `value` under `key`, replacing any existing record, and persist.
    fn put(&mut self, key: &str, value: &str) -> Result<(), SecretError>;
    /// Remove a record and persist. Missing key is not an error.
    fn delete(&mut self, key: &str) -> Result<(), SecretError>;
    /// Record keys currently present (no values decrypted).
    fn list_keys(&self) -> Vec<String>;
}
