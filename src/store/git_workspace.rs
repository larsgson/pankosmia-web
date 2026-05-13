//! `GitWorkspace` — abstraction over the on-disk Git working tree.
//!
//! Important: this trait is filesystem-shaped in **both** FS and
//! Supabase backends. `git2` is sync and wants a real working tree
//! somewhere on disk. The Supabase backend caches working copies per
//! request rather than trying to wire git operations through
//! Postgres.
//!
//! All `git2` operations must run inside `tokio::task::spawn_blocking`
//! (or a dedicated bounded thread pool — see `SCALING.md` §3.3).
//! That contract is enforced by the implementation, not the trait.
//!
//! Trait skeleton only here; full implementations land in M3
//! (FS) and beyond.

use crate::identity::{LanguageCode, RepoId};
use crate::store::types::*;
use async_trait::async_trait;
use std::path::PathBuf;

/// Resolved pointer to a repo's working tree on local disk.
pub struct WorkingCopy {
    pub language: LanguageCode,
    pub repo: RepoId,
    pub path: PathBuf,
}

#[async_trait]
pub trait GitWorkspace: Send + Sync {
    /// Resolve `(language, repo)` to a working-copy handle. The
    /// handle's `path` is guaranteed to be inside the workspace root
    /// (path-traversal-validated by the implementation). May lazily
    /// clone / fetch in hosted deployments.
    async fn working_copy(&self, lang: LanguageCode, repo: RepoId) -> StoreResult<WorkingCopy>;
}
