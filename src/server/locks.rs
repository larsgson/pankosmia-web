//! Per-language `RwLock` map.
//!
//! `git2` is synchronous and serializes commits on the same repo.
//! Without per-language locking, every git operation across the
//! server contends on the OS-level git index lock. With per-language
//! locking, concurrent operations on different languages don't
//! contend at all; concurrent operations on one language serialize
//! at the application layer where we can hold the lock across
//! related FS work (write blob, update metadata, commit).
//!
//! Map shape: outer `parking_lot::RwLock` for cheap concurrent map
//! access (lookup happens on every per-language request); inner
//! `tokio::sync::RwLock` for the actual per-language semaphore so
//! `await`s don't block worker threads.
//!
//! Memory footprint: ~80 bytes per language entry, lazily created on
//! first use. 1,000 active languages = 80 KB.

use crate::identity::LanguageCode;
use parking_lot::RwLock as PlRwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock as TokioRwLock;

pub struct LanguageLocks {
    inner: PlRwLock<HashMap<LanguageCode, Arc<TokioRwLock<()>>>>,
}

impl LanguageLocks {
    pub fn new() -> Self {
        Self {
            inner: PlRwLock::new(HashMap::new()),
        }
    }

    /// Acquire (or lazily create) the lock for a language. Caller
    /// then `.read()` or `.write()` on the returned `Arc` to
    /// serialize work.
    pub fn for_language(&self, lang: &LanguageCode) -> Arc<TokioRwLock<()>> {
        if let Some(lock) = self.inner.read().get(lang) {
            return lock.clone();
        }
        let mut w = self.inner.write();
        w.entry(lang.clone())
            .or_insert_with(|| Arc::new(TokioRwLock::new(())))
            .clone()
    }

    /// Diagnostics: how many languages have ever been locked. Used
    /// by metrics endpoints; cheap to call.
    pub fn known_language_count(&self) -> usize {
        self.inner.read().len()
    }
}

impl Default for LanguageLocks {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn for_language_returns_same_arc_per_language() {
        let locks = LanguageLocks::new();
        let en = LanguageCode::parse("en").unwrap();
        let a = locks.for_language(&en);
        let b = locks.for_language(&en);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(locks.known_language_count(), 1);
    }

    #[tokio::test]
    async fn different_languages_get_different_locks() {
        let locks = LanguageLocks::new();
        let en = LanguageCode::parse("en").unwrap();
        let fr = LanguageCode::parse("fr").unwrap();
        let a = locks.for_language(&en);
        let b = locks.for_language(&fr);
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(locks.known_language_count(), 2);
    }

    #[tokio::test]
    async fn read_locks_dont_serialize() {
        let locks = LanguageLocks::new();
        let en = LanguageCode::parse("en").unwrap();
        let lock = locks.for_language(&en);
        let _r1 = lock.read().await;
        // Multiple readers OK simultaneously.
        let _r2 = lock.read().await;
    }
}
