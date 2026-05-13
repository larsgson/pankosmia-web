//! `ProjectStore` — the central abstraction over per-language state.
//!
//! Two implementations:
//!   - `FsLanguageStore` for single-tenant FS deployments (M1+).
//!   - `SupabaseLanguageStore` for hosted multi-tenant deployments
//!     (M6+).
//!
//! Endpoints call only this trait, never `std::fs::*` directly. The
//! Phase 2 design contract is in `docs/PHASE2_DESIGN.md`.

use crate::identity::{LanguageCode, RepoId, UserId};
use crate::store::types::*;
use async_trait::async_trait;
use futures::future::BoxFuture;
use std::path::{Path, PathBuf};

#[async_trait]
pub trait ProjectStore: Send + Sync {
    // --- identity & membership -------------------------------------

    /// All language memberships for a user.
    async fn list_user_languages(&self, user: UserId) -> StoreResult<Vec<ProjectSummary>>;

    /// Membership role of `user` on `lang`, or `None` if not a
    /// member. Hot path — implementations should be cheap.
    async fn project_role(&self, user: UserId, lang: LanguageCode) -> StoreResult<Option<Role>>;

    /// Bootstrap a new language. Reserved for admin / migration
    /// tooling.
    async fn create_project(&self, owner: UserId, spec: NewProject) -> StoreResult<()>;

    async fn add_member(&self, lang: LanguageCode, user: UserId, role: Role) -> StoreResult<()>;
    async fn remove_member(&self, lang: LanguageCode, user: UserId) -> StoreResult<()>;

    // --- per-user settings -----------------------------------------

    async fn get_user_settings(&self, user: UserId) -> StoreResult<UserSettings>;
    async fn put_user_settings(&self, user: UserId, s: UserSettings) -> StoreResult<()>;
    async fn get_languages(&self, user: UserId) -> StoreResult<Vec<LanguageCode>>;
    async fn put_languages(&self, user: UserId, langs: Vec<LanguageCode>) -> StoreResult<()>;
    async fn get_typography(&self, user: UserId) -> StoreResult<Typography>;
    async fn put_typography(&self, user: UserId, t: Typography) -> StoreResult<()>;

    // --- per-language app state ------------------------------------

    async fn get_app_state(&self, lang: LanguageCode) -> StoreResult<AppState>;
    async fn put_app_state(&self, lang: LanguageCode, s: AppState) -> StoreResult<()>;

    /// Per-(user, language) cursor.
    async fn get_bcv(&self, lang: LanguageCode, user: UserId) -> StoreResult<Bcv>;
    async fn put_bcv(&self, lang: LanguageCode, user: UserId, bcv: Bcv) -> StoreResult<()>;

    // --- gitea OAuth (per-user) ------------------------------------

    async fn get_auth_token(&self, user: UserId, key: &str) -> StoreResult<Option<String>>;
    async fn put_auth_token(&self, user: UserId, key: &str, code: &str) -> StoreResult<()>;
    async fn delete_auth_token(&self, user: UserId, key: &str) -> StoreResult<()>;
    async fn put_auth_request(&self, user: UserId, key: &str, req: AuthRequest) -> StoreResult<()>;
    async fn take_auth_request(&self, user: UserId, key: &str) -> StoreResult<Option<AuthRequest>>;

    // --- repo registry --------------------------------------------

    async fn list_repos(&self, lang: LanguageCode) -> StoreResult<Vec<RepoRecord>>;
    async fn register_repo(&self, lang: LanguageCode, r: NewRepo) -> StoreResult<RepoId>;
    async fn unregister_repo(&self, lang: LanguageCode, repo: RepoId) -> StoreResult<()>;
    async fn lookup_repo(&self, lang: LanguageCode, repo: RepoId) -> StoreResult<RepoRecord>;

    // --- burrito metadata -----------------------------------------
    //
    // Note: passing both `lang` and `repo` is deliberate even though
    // `RepoId` is globally unique — keeps the FS implementation able
    // to resolve directly without a reverse-index lookup. Supabase
    // implementation can ignore the redundant `lang` parameter.

    async fn get_burrito_metadata(
        &self,
        lang: LanguageCode,
        repo: RepoId,
    ) -> StoreResult<BurritoMetadata>;
    async fn put_burrito_metadata(
        &self,
        lang: LanguageCode,
        repo: RepoId,
        m: BurritoMetadata,
    ) -> StoreResult<()>;
    async fn list_ingredient_summaries(
        &self,
        lang: LanguageCode,
        repo: RepoId,
    ) -> StoreResult<Vec<IngredientSummary>>;

    // --- legacy repo path resolution ------------------------------
    //
    // Resolve a legacy `<source>/<org>/<name>` repo path string to an
    // absolute filesystem path under the workspace root. Validates
    // each segment against path-traversal rules and rejects the
    // reserved `.pankosmia/` prefix.
    //
    // This is the M3 seam: every endpoint that previously did
    // `format!("{}/{}", state.repo_dir.lock(), repo_path)` calls this
    // instead. The implementation centralises path-traversal defense
    // and isolates the rest of the codebase from the workspace-root
    // detail.
    //
    // UUID-keyed working trees (`<lang>/<repo_id>/`) are a Phase 2
    // concern; until then this method returns paths in the legacy
    // `<workspace_root>/<source>/<org>/<name>/` layout.
    async fn repo_workspace_path(&self, repo_path: &Path) -> StoreResult<PathBuf>;

    /// Workspace root itself, for endpoints that need to enumerate
    /// repos (e.g. `list_local_repos`). Implementations MUST keep
    /// this stable across the lifetime of the process.
    fn workspace_root(&self) -> &Path;

    /// Refresh local cache for a language (clone if missing, fetch
    /// otherwise). Called by the language webhook to mirror an
    /// upstream merge into the shared read cache, so SSE subscribers
    /// see the updated mtimes via `WatcherRegistry`. FS implementations
    /// are a no-op (no upstream to fetch).
    async fn prefetch_language(&self, _lang: LanguageCode) -> StoreResult<()> {
        Ok(())
    }

    // --- multi-write atomicity ------------------------------------
    //
    // The FS impl is a no-op wrapper (single-process, single-thread
    // git contention is handled by `LanguageLocks`, not transactions).
    // The Supabase impl runs a real Postgres transaction.

    async fn with_tx<'a>(
        &'a self,
        f: Box<
            dyn for<'t> FnOnce(&'t mut (dyn Tx + 'a)) -> BoxFuture<'t, StoreResult<()>> + Send + 'a,
        >,
    ) -> StoreResult<()>;
}

/// Mutating operations available inside `with_tx`. Mirrors a small
/// subset of `ProjectStore` — the operations that can compose into a
/// single atomic write.
#[async_trait]
pub trait Tx: Send {
    async fn put_app_state(&mut self, lang: LanguageCode, s: AppState) -> StoreResult<()>;
    async fn put_burrito_metadata(
        &mut self,
        lang: LanguageCode,
        repo: RepoId,
        m: BurritoMetadata,
    ) -> StoreResult<()>;
}
