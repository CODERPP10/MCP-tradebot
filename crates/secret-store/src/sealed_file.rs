//! [`SealedFileStore`] — the v1 [`SecretStore`](crate::SecretStore).
//!
//! On-disk layout (`serde_json`):
//!
//! ```json
//! {
//!   "version": 1,
//!   "kdf": { "algo": "argon2id", "m_cost_kib": 19456, "t_cost": 2,
//!            "p_cost": 1, "salt_b64": "…" },
//!   "verifier_b64": "…",
//!   "records": { "<key>": { "nonce_b64": "…", "ct_b64": "…" } }
//! }
//! ```
//!
//! Each record is sealed with XChaCha20-Poly1305; the record key is passed as
//! AEAD associated data, so a record cannot be silently moved to another key.
//! The `verifier_b64` entry seals a fixed marker under the AAD `"__verifier__"`,
//! letting [`SealedFileStore::open`] report a wrong passphrase immediately
//! rather than on the first `get`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{SecretError, SecretStore};

const FORMAT_VERSION: u32 = 1;
const VERIFIER_AAD: &[u8] = b"__verifier__";
const VERIFIER_PLAINTEXT: &[u8] = b"tradebot-secret-store-v1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;

/// Argon2id cost parameters, recorded in the file so [`SealedFileStore::open`]
/// can reproduce the key.
#[derive(Debug, Clone, Copy)]
pub struct Argon2Params {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for Argon2Params {
    /// OWASP-recommended Argon2id baseline (19 MiB, 2 iterations, 1 lane).
    fn default() -> Self {
        Self {
            m_cost_kib: 19 * 1024,
            t_cost: 2,
            p_cost: 1,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct FileFormat {
    version: u32,
    kdf: KdfHeader,
    verifier_b64: String,
    records: BTreeMap<String, SealedRecord>,
}

#[derive(Serialize, Deserialize)]
struct KdfHeader {
    algo: String,
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
    salt_b64: String,
}

#[derive(Serialize, Deserialize)]
struct SealedRecord {
    nonce_b64: String,
    ct_b64: String,
}

/// A passphrase-sealed secret file. Holds the derived cipher in memory (zeroized
/// on drop by `chacha20poly1305`); the passphrase is not retained.
pub struct SealedFileStore {
    path: PathBuf,
    cipher: XChaCha20Poly1305,
    kdf: KdfHeader,
    records: BTreeMap<String, SealedRecord>,
}

impl SealedFileStore {
    /// Create a brand-new sealed store at `path`. Errors if the file exists.
    pub fn create(
        path: impl Into<PathBuf>,
        passphrase: &str,
        params: Argon2Params,
    ) -> Result<Self, SecretError> {
        let path = path.into();
        if path.exists() {
            return Err(SecretError::Io(format!(
                "{} already exists; refusing to overwrite",
                path.display()
            )));
        }
        let mut salt = [0u8; SALT_LEN];
        rand::thread_rng().fill_bytes(&mut salt);
        let cipher = derive_cipher(passphrase, &salt, &params)?;

        let kdf = KdfHeader {
            algo: "argon2id".to_string(),
            m_cost_kib: params.m_cost_kib,
            t_cost: params.t_cost,
            p_cost: params.p_cost,
            salt_b64: B64.encode(salt),
        };
        let store = Self {
            path,
            cipher,
            kdf,
            records: BTreeMap::new(),
        };
        store.persist()?;
        Ok(store)
    }

    /// Open an existing sealed store. Returns [`SecretError::Unsealing`] if the
    /// passphrase is wrong or the file has been tampered with.
    pub fn open(path: impl Into<PathBuf>, passphrase: &str) -> Result<Self, SecretError> {
        let path = path.into();
        let raw = fs::read_to_string(&path).map_err(|e| SecretError::Io(e.to_string()))?;
        let file: FileFormat =
            serde_json::from_str(&raw).map_err(|e| SecretError::Corrupt(e.to_string()))?;
        if file.version != FORMAT_VERSION {
            return Err(SecretError::Corrupt(format!(
                "unsupported store version {}",
                file.version
            )));
        }
        if file.kdf.algo != "argon2id" {
            return Err(SecretError::Corrupt(format!(
                "unknown kdf {}",
                file.kdf.algo
            )));
        }
        let salt = B64
            .decode(&file.kdf.salt_b64)
            .map_err(|e| SecretError::Corrupt(format!("salt: {e}")))?;
        let params = Argon2Params {
            m_cost_kib: file.kdf.m_cost_kib,
            t_cost: file.kdf.t_cost,
            p_cost: file.kdf.p_cost,
        };
        let cipher = derive_cipher(passphrase, &salt, &params)?;

        // Passphrase / integrity check.
        let verifier_ct = B64
            .decode(&file.verifier_b64)
            .map_err(|e| SecretError::Corrupt(format!("verifier: {e}")))?;
        let (v_nonce, v_body) = split_nonce(&verifier_ct)?;
        let plain = cipher
            .decrypt(
                v_nonce,
                Payload {
                    msg: v_body,
                    aad: VERIFIER_AAD,
                },
            )
            .map_err(|_| SecretError::Unsealing)?;
        if plain != VERIFIER_PLAINTEXT {
            return Err(SecretError::Unsealing);
        }

        Ok(Self {
            path,
            cipher,
            kdf: file.kdf,
            records: file.records,
        })
    }

    fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<SealedRecord, SecretError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);
        let ct = self
            .cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| SecretError::Invalid("encryption failed".to_string()))?;
        Ok(SealedRecord {
            nonce_b64: B64.encode(nonce_bytes),
            ct_b64: B64.encode(ct),
        })
    }

    fn open_record(&self, key: &str, rec: &SealedRecord) -> Result<String, SecretError> {
        let nonce = B64
            .decode(&rec.nonce_b64)
            .map_err(|e| SecretError::Corrupt(format!("{key} nonce: {e}")))?;
        let ct = B64
            .decode(&rec.ct_b64)
            .map_err(|e| SecretError::Corrupt(format!("{key} ct: {e}")))?;
        if nonce.len() != NONCE_LEN {
            return Err(SecretError::Corrupt(format!("{key}: bad nonce length")));
        }
        let plain = self
            .cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ct,
                    aad: key.as_bytes(),
                },
            )
            .map_err(|_| SecretError::Unsealing)?;
        String::from_utf8(plain).map_err(|_| SecretError::Corrupt(format!("{key}: not utf-8")))
    }

    fn persist(&self) -> Result<(), SecretError> {
        let verifier = self.seal(VERIFIER_AAD, VERIFIER_PLAINTEXT)?;
        // verifier is stored as nonce||ct so it is self-contained.
        let verifier_blob = {
            let mut v = B64.decode(&verifier.nonce_b64).unwrap();
            v.extend_from_slice(&B64.decode(&verifier.ct_b64).unwrap());
            B64.encode(v)
        };
        let file = FileFormat {
            version: FORMAT_VERSION,
            kdf: KdfHeader {
                algo: self.kdf.algo.clone(),
                m_cost_kib: self.kdf.m_cost_kib,
                t_cost: self.kdf.t_cost,
                p_cost: self.kdf.p_cost,
                salt_b64: self.kdf.salt_b64.clone(),
            },
            verifier_b64: verifier_blob,
            records: self
                .records
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        SealedRecord {
                            nonce_b64: v.nonce_b64.clone(),
                            ct_b64: v.ct_b64.clone(),
                        },
                    )
                })
                .collect(),
        };
        let json =
            serde_json::to_string_pretty(&file).map_err(|e| SecretError::Corrupt(e.to_string()))?;

        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, json.as_bytes()).map_err(|e| SecretError::Io(e.to_string()))?;
        set_owner_only(&tmp);
        // std::fs::rename replaces an existing destination atomically on both
        // Unix (rename(2)) and Windows (MoveFileExW + REPLACE_EXISTING).
        fs::rename(&tmp, &self.path).map_err(|e| SecretError::Io(e.to_string()))?;
        set_owner_only(&self.path);
        Ok(())
    }
}

impl SecretStore for SealedFileStore {
    fn get(&self, key: &str) -> Result<String, SecretError> {
        let rec = self
            .records
            .get(key)
            .ok_or_else(|| SecretError::NotFound(key.to_string()))?;
        self.open_record(key, rec)
    }

    fn put(&mut self, key: &str, value: &str) -> Result<(), SecretError> {
        if key.is_empty() || key == "__verifier__" {
            return Err(SecretError::Invalid(format!("reserved key {key:?}")));
        }
        let rec = self.seal(key.as_bytes(), value.as_bytes())?;
        let prev = self.records.insert(key.to_string(), rec);
        if let Err(e) = self.persist() {
            // roll back the in-memory change so state matches disk
            match prev {
                Some(p) => {
                    self.records.insert(key.to_string(), p);
                }
                None => {
                    self.records.remove(key);
                }
            }
            return Err(e);
        }
        Ok(())
    }

    fn delete(&mut self, key: &str) -> Result<(), SecretError> {
        if let Some(prev) = self.records.remove(key) {
            if let Err(e) = self.persist() {
                self.records.insert(key.to_string(), prev);
                return Err(e);
            }
        }
        Ok(())
    }

    fn list_keys(&self) -> Vec<String> {
        self.records.keys().cloned().collect()
    }
}

fn derive_cipher(
    passphrase: &str,
    salt: &[u8],
    params: &Argon2Params,
) -> Result<XChaCha20Poly1305, SecretError> {
    if salt.len() < 8 {
        return Err(SecretError::Corrupt("salt too short".to_string()));
    }
    let p = Params::new(params.m_cost_kib, params.t_cost, params.p_cost, Some(32))
        .map_err(|e| SecretError::Corrupt(format!("argon2 params: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut_slice())
        .map_err(|_| SecretError::Unsealing)?;
    Ok(XChaCha20Poly1305::new_from_slice(key.as_slice()).expect("32-byte key"))
}

fn split_nonce(blob: &[u8]) -> Result<(&XNonce, &[u8]), SecretError> {
    if blob.len() <= NONCE_LEN {
        return Err(SecretError::Corrupt("sealed blob too short".to_string()));
    }
    let (n, body) = blob.split_at(NONCE_LEN);
    Ok((XNonce::from_slice(n), body))
}

#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KITE_API_SECRET;

    fn test_params() -> Argon2Params {
        // Minimum-ish cost so tests are fast; NOT for production.
        Argon2Params {
            m_cost_kib: 16,
            t_cost: 1,
            p_cost: 1,
        }
    }

    /// Returns a temp dir (keep it in scope so it is cleaned up) and the store
    /// path inside it.
    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        (dir, path)
    }

    #[test]
    fn create_put_reopen_get_roundtrips() {
        let (_d, path) = tmp();
        {
            let mut s = SealedFileStore::create(&path, "correct horse", test_params()).unwrap();
            s.put(KITE_API_SECRET, "s3cr3t-value").unwrap();
            s.put("telegram_chat_id", "12345").unwrap();
        }
        let s = SealedFileStore::open(&path, "correct horse").unwrap();
        assert_eq!(s.get(KITE_API_SECRET).unwrap(), "s3cr3t-value");
        assert_eq!(s.get("telegram_chat_id").unwrap(), "12345");
        let mut keys = s.list_keys();
        keys.sort();
        assert_eq!(keys, vec!["kite_api_secret", "telegram_chat_id"]);
    }

    #[test]
    fn wrong_passphrase_is_rejected_on_open() {
        let (_d, path) = tmp();
        SealedFileStore::create(&path, "right", test_params())
            .unwrap()
            .put("k", "v")
            .unwrap();
        assert!(matches!(
            SealedFileStore::open(&path, "wrong"),
            Err(SecretError::Unsealing)
        ));
    }

    #[test]
    fn plaintext_never_appears_on_disk() {
        let (_d, path) = tmp();
        let mut s = SealedFileStore::create(&path, "pp", test_params()).unwrap();
        s.put(KITE_API_SECRET, "TOPSECRET_NEEDLE").unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("TOPSECRET_NEEDLE"));
        assert!(!raw.contains("pp")); // passphrase not persisted
    }

    #[test]
    fn tampering_with_a_record_is_detected() {
        let (_d, path) = tmp();
        {
            let mut s = SealedFileStore::create(&path, "pp", test_params()).unwrap();
            s.put("k", "v").unwrap();
        }
        let mut file: FileFormat =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        // flip a byte of the ciphertext
        let rec = file.records.get_mut("k").unwrap();
        let mut ct = B64.decode(&rec.ct_b64).unwrap();
        ct[0] ^= 0xff;
        rec.ct_b64 = B64.encode(ct);
        fs::write(&path, serde_json::to_string(&file).unwrap()).unwrap();

        let s = SealedFileStore::open(&path, "pp").unwrap();
        assert!(matches!(s.get("k"), Err(SecretError::Unsealing)));
    }

    #[test]
    fn record_cannot_be_moved_to_another_key() {
        let (_d, path) = tmp();
        {
            let mut s = SealedFileStore::create(&path, "pp", test_params()).unwrap();
            s.put("a", "value-a").unwrap();
        }
        let mut file: FileFormat =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let rec_a = file.records.remove("a").unwrap();
        file.records.insert("b".to_string(), rec_a); // same ciphertext, new key
        fs::write(&path, serde_json::to_string(&file).unwrap()).unwrap();

        let s = SealedFileStore::open(&path, "pp").unwrap();
        // AAD mismatch (key "b" != sealed-under "a") ⇒ auth failure
        assert!(matches!(s.get("b"), Err(SecretError::Unsealing)));
    }

    #[test]
    fn delete_removes_and_persists() {
        let (_d, path) = tmp();
        let mut s = SealedFileStore::create(&path, "pp", test_params()).unwrap();
        s.put("k", "v").unwrap();
        s.delete("k").unwrap();
        assert!(matches!(s.get("k"), Err(SecretError::NotFound(_))));
        let s2 = SealedFileStore::open(&path, "pp").unwrap();
        assert!(s2.list_keys().is_empty());
    }
}
