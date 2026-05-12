//! JWKS (JSON Web Key Set) cache for Supabase JWT verification.
//!
//! Fetches Supabase's RS256 public keys from the configured JWKS
//! endpoint and caches them for 5 minutes. The cache survives
//! transient JWKS-endpoint downtime — verification continues with
//! cached keys until the TTL elapses, then a fresh fetch is required
//! before any new token can be verified.
//!
//! The cache is keyed by the JWT header's `kid` claim. A cache miss
//! triggers a refresh; if `kid` is still unknown after refresh, the
//! verification fails with `JwksError::UnknownKid`.

use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
    #[allow(dead_code)]
    alg: Option<String>,
    #[allow(dead_code)]
    kty: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

pub struct JwksCache {
    url: String,
    ttl: Duration,
    inner: Mutex<Option<CachedSet>>,
}

struct CachedSet {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Instant,
}

impl JwksCache {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ttl: Duration::from_secs(300),
            inner: Mutex::new(None),
        }
    }

    /// Decoding key for a `kid`. Returns the cached key if fresh;
    /// otherwise refreshes from the JWKS endpoint and tries again.
    pub async fn key_for(&self, kid: &str) -> Result<DecodingKey, JwksError> {
        if let Some(k) = self.lookup_fresh(kid) {
            return Ok(k);
        }
        self.refresh().await?;
        self.lookup_fresh(kid)
            .ok_or_else(|| JwksError::UnknownKid(kid.to_string()))
    }

    fn lookup_fresh(&self, kid: &str) -> Option<DecodingKey> {
        let guard = self.inner.lock().unwrap();
        let c = guard.as_ref()?;
        if c.fetched_at.elapsed() > self.ttl {
            return None;
        }
        c.keys.get(kid).cloned()
    }

    async fn refresh(&self) -> Result<(), JwksError> {
        let resp = reqwest::get(&self.url)
            .await
            .map_err(|e| JwksError::Fetch(e.to_string()))?;
        let set: JwkSet = resp
            .json()
            .await
            .map_err(|e| JwksError::Fetch(e.to_string()))?;
        let mut keys = HashMap::new();
        for jwk in set.keys {
            let key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
                .map_err(|e| JwksError::Decode(e.to_string()))?;
            keys.insert(jwk.kid, key);
        }
        *self.inner.lock().unwrap() = Some(CachedSet {
            keys,
            fetched_at: Instant::now(),
        });
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JwksError {
    #[error("fetch JWKS: {0}")]
    Fetch(String),
    #[error("decode JWKS: {0}")]
    Decode(String),
    #[error("unknown kid: {0}")]
    UnknownKid(String),
}
