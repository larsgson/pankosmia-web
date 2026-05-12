//! Encrypted on-disk storage for per-user GitHub OAuth tokens.
//!
//! Tokens at rest are AES-GCM-encrypted with a server-held 32-byte
//! key (env var `PANKOSMIA_TOKEN_ENCRYPTION_KEY`, base64). Each
//! token gets a fresh random nonce so identical tokens encrypt to
//! different ciphertext.
//!
//! Layout:
//!
//! ```text
//! <workspace_root>/.pankosmia/users/<github_user_id>/token.bin
//! ```
//!
//! File contents: `nonce(12) || ciphertext(*)`.
//!
//! If `PANKOSMIA_TOKEN_ENCRYPTION_KEY` is not set, the server uses
//! a per-process random key — fine for dev but means every restart
//! invalidates all stored tokens. Print a warning at startup if so.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum TokenStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("token format: {0}")]
    Format(String),
}

pub struct TokenStore {
    root: PathBuf,
    cipher: Aes256Gcm,
}

impl TokenStore {
    pub fn new(workspace_root: PathBuf, key: [u8; 32]) -> Self {
        let cipher = Aes256Gcm::new(&key.into());
        Self {
            root: workspace_root,
            cipher,
        }
    }

    /// Build a `TokenStore` reading the encryption key from
    /// `PANKOSMIA_TOKEN_ENCRYPTION_KEY` (base64). On miss, use a
    /// random per-process key and print a warning.
    pub fn from_env(workspace_root: PathBuf) -> Self {
        let key = match std::env::var("PANKOSMIA_TOKEN_ENCRYPTION_KEY").ok() {
            Some(b64) => match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                Ok(b) if b.len() == 32 => {
                    let mut k = [0u8; 32];
                    k.copy_from_slice(&b);
                    k
                }
                Ok(_) => {
                    eprintln!(
                        "WARN: PANKOSMIA_TOKEN_ENCRYPTION_KEY must decode to 32 bytes; \
                         falling back to per-process random key"
                    );
                    random_key()
                }
                Err(e) => {
                    eprintln!(
                        "WARN: PANKOSMIA_TOKEN_ENCRYPTION_KEY not valid base64 ({}); \
                         falling back to per-process random key",
                        e
                    );
                    random_key()
                }
            },
            None => {
                eprintln!(
                    "WARN: PANKOSMIA_TOKEN_ENCRYPTION_KEY not set; using per-process random key. \
                     Stored tokens will be invalid after restart."
                );
                random_key()
            }
        };
        Self::new(workspace_root, key)
    }

    fn token_path(&self, github_user_id: i64) -> PathBuf {
        self.root
            .join(".pankosmia")
            .join("users")
            .join(github_user_id.to_string())
            .join("token.bin")
    }

    pub fn save(
        &self,
        github_user_id: i64,
        access_token: &str,
    ) -> Result<(), TokenStoreError> {
        let nonce_bytes = random_nonce();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, access_token.as_bytes())
            .map_err(|e| TokenStoreError::Crypto(format!("encrypt: {}", e)))?;
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);
        let path = self.token_path(github_user_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, blob)?;
        Ok(())
    }

    pub fn load(&self, github_user_id: i64) -> Result<Option<String>, TokenStoreError> {
        let path = self.token_path(github_user_id);
        if !path.exists() {
            return Ok(None);
        }
        let blob = std::fs::read(&path)?;
        if blob.len() < 12 {
            return Err(TokenStoreError::Format("token blob too short".into()));
        }
        let (nonce_bytes, ciphertext) = blob.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| TokenStoreError::Crypto(format!("decrypt: {}", e)))?;
        let s = String::from_utf8(plaintext)
            .map_err(|e| TokenStoreError::Format(format!("utf8: {}", e)))?;
        Ok(Some(s))
    }

    pub fn delete(&self, github_user_id: i64) -> Result<(), TokenStoreError> {
        let path = self.token_path(github_user_id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn random_key() -> [u8; 32] {
    use aes_gcm::aead::OsRng;
    use aes_gcm::aead::rand_core::RngCore;
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

fn random_nonce() -> [u8; 12] {
    use aes_gcm::aead::OsRng;
    use aes_gcm::aead::rand_core::RngCore;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = TokenStore::new(tmp.path().to_path_buf(), [7u8; 32]);
        store.save(42, "ghs_secret_token").unwrap();
        let got = store.load(42).unwrap();
        assert_eq!(got.as_deref(), Some("ghs_secret_token"));
    }

    #[test]
    fn missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let store = TokenStore::new(tmp.path().to_path_buf(), [7u8; 32]);
        assert!(store.load(99).unwrap().is_none());
    }

    #[test]
    fn delete_removes_file() {
        let tmp = TempDir::new().unwrap();
        let store = TokenStore::new(tmp.path().to_path_buf(), [7u8; 32]);
        store.save(1, "x").unwrap();
        store.delete(1).unwrap();
        assert!(store.load(1).unwrap().is_none());
    }

    #[test]
    fn different_keys_dont_decrypt() {
        let tmp = TempDir::new().unwrap();
        let store_a = TokenStore::new(tmp.path().to_path_buf(), [1u8; 32]);
        let store_b = TokenStore::new(tmp.path().to_path_buf(), [2u8; 32]);
        store_a.save(1, "x").unwrap();
        assert!(store_b.load(1).is_err());
    }
}
