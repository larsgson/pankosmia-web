//! Helpers that combine `LanguageLocks` + `BlockingPools` for git
//! endpoints.
//!
//! Every git endpoint follows the same pattern:
//!
//!   1. Acquire the per-language lock (read for status/log/list,
//!      write for clone/commit/push/pull/branch).
//!   2. Dispatch the synchronous `git2` work onto the bounded git
//!      pool so it doesn't block Tokio worker threads.
//!   3. Map JoinHandle / pool errors to a single result type.
//!
//! These helpers wrap that pattern. The endpoint body becomes:
//!
//! ```rust,ignore
//! match git_dispatch::run_locked_write(locks, pools, lang, move || {
//!     // synchronous git2 work, returning Result<R, String>
//! }).await {
//!     Ok(r) => ok_response(r),
//!     Err(e) => error_response(e),
//! }
//! ```
//!
//! `run_locked_read` is the equivalent for read-only operations
//! (`status`, `log`, `branches`, `remotes`).

use crate::identity::LanguageCode;
use crate::server::{BlockingPools, LanguageLocks};

/// Errors that can come from the dispatch path itself (separate
/// from errors inside the user's git2 closure).
#[derive(Debug, thiserror::Error)]
pub enum GitOpError {
    #[error("git pool full / shutdown")]
    PoolAcquire,
    #[error("git task panicked: {0}")]
    Panic(String),
    #[error("{0}")]
    Git(String),
}

/// Take a per-language **write** lock, then dispatch a synchronous
/// `git2` closure onto the bounded git pool. The lock is held for
/// the entire duration of the git work — concurrent writes on the
/// same language serialize here. Concurrent writes on different
/// languages don't block each other.
pub async fn run_locked_write<F, R>(
    locks: &LanguageLocks,
    pools: &BlockingPools,
    lang: &LanguageCode,
    f: F,
) -> Result<R, GitOpError>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    let lock = locks.for_language(lang);
    let _guard = lock.write().await;
    let join = pools
        .run_git(f)
        .await
        .map_err(|_| GitOpError::PoolAcquire)?;
    let inner = join.await.map_err(|e| GitOpError::Panic(e.to_string()))?;
    inner.map_err(GitOpError::Git)
}

/// Take a per-language **read** lock, dispatch onto the git pool.
/// Multiple concurrent reads on the same language don't serialize;
/// they only block when a writer is active.
pub async fn run_locked_read<F, R>(
    locks: &LanguageLocks,
    pools: &BlockingPools,
    lang: &LanguageCode,
    f: F,
) -> Result<R, GitOpError>
where
    F: FnOnce() -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    let lock = locks.for_language(lang);
    let _guard = lock.read().await;
    let join = pools
        .run_git(f)
        .await
        .map_err(|_| GitOpError::PoolAcquire)?;
    let inner = join.await.map_err(|e| GitOpError::Panic(e.to_string()))?;
    inner.map_err(GitOpError::Git)
}
