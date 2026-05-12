//! `SupabaseLanguageStore` — Phase 2 hosted backend skeleton.
//!
//! Every method returns `StoreError::Backend("not implemented")`.
//! M7 fills in memberships + user settings; M8 fills in repos +
//! burrito metadata. The skeleton ships first so the migration
//! infrastructure, runtime selector, and CI matrix can operate
//! against a real (if non-functional) impl.

use crate::identity::{LanguageCode, RepoId, UserId};
use crate::store::project_store::{ProjectStore, Tx};
use crate::store::types::*;
use async_trait::async_trait;
use futures::future::BoxFuture;
use sqlx::PgPool;

pub struct SupabaseLanguageStore {
    pool: PgPool,
}

impl SupabaseLanguageStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

fn nyi(method: &'static str) -> StoreError {
    StoreError::Backend(format!("SupabaseLanguageStore::{}: not implemented", method))
}

#[async_trait]
impl ProjectStore for SupabaseLanguageStore {
    // --- identity & membership (M7) -------------------------------

    async fn list_user_languages(
        &self,
        _user: UserId,
    ) -> StoreResult<Vec<ProjectSummary>> {
        Err(nyi("list_user_languages"))
    }
    async fn project_role(
        &self,
        _user: UserId,
        _lang: LanguageCode,
    ) -> StoreResult<Option<Role>> {
        Err(nyi("project_role"))
    }
    async fn create_project(
        &self,
        _owner: UserId,
        _spec: NewProject,
    ) -> StoreResult<()> {
        Err(nyi("create_project"))
    }
    async fn add_member(
        &self,
        _lang: LanguageCode,
        _user: UserId,
        _role: Role,
    ) -> StoreResult<()> {
        Err(nyi("add_member"))
    }
    async fn remove_member(
        &self,
        _lang: LanguageCode,
        _user: UserId,
    ) -> StoreResult<()> {
        Err(nyi("remove_member"))
    }

    // --- per-user settings (M7) -----------------------------------

    async fn get_user_settings(&self, _user: UserId) -> StoreResult<UserSettings> {
        Err(nyi("get_user_settings"))
    }
    async fn put_user_settings(
        &self,
        _user: UserId,
        _s: UserSettings,
    ) -> StoreResult<()> {
        Err(nyi("put_user_settings"))
    }
    async fn get_languages(&self, _user: UserId) -> StoreResult<Vec<LanguageCode>> {
        Err(nyi("get_languages"))
    }
    async fn put_languages(
        &self,
        _user: UserId,
        _langs: Vec<LanguageCode>,
    ) -> StoreResult<()> {
        Err(nyi("put_languages"))
    }
    async fn get_typography(&self, _user: UserId) -> StoreResult<Typography> {
        Err(nyi("get_typography"))
    }
    async fn put_typography(&self, _user: UserId, _t: Typography) -> StoreResult<()> {
        Err(nyi("put_typography"))
    }

    // --- per-language app state (M7) ------------------------------

    async fn get_app_state(&self, _lang: LanguageCode) -> StoreResult<AppState> {
        Err(nyi("get_app_state"))
    }
    async fn put_app_state(
        &self,
        _lang: LanguageCode,
        _s: AppState,
    ) -> StoreResult<()> {
        Err(nyi("put_app_state"))
    }
    async fn get_bcv(
        &self,
        _lang: LanguageCode,
        _user: UserId,
    ) -> StoreResult<Bcv> {
        Err(nyi("get_bcv"))
    }
    async fn put_bcv(
        &self,
        _lang: LanguageCode,
        _user: UserId,
        _bcv: Bcv,
    ) -> StoreResult<()> {
        Err(nyi("put_bcv"))
    }

    // --- gitea OAuth (M7) -----------------------------------------

    async fn get_auth_token(
        &self,
        _user: UserId,
        _key: &str,
    ) -> StoreResult<Option<String>> {
        Err(nyi("get_auth_token"))
    }
    async fn put_auth_token(
        &self,
        _user: UserId,
        _key: &str,
        _code: &str,
    ) -> StoreResult<()> {
        Err(nyi("put_auth_token"))
    }
    async fn delete_auth_token(&self, _user: UserId, _key: &str) -> StoreResult<()> {
        Err(nyi("delete_auth_token"))
    }
    async fn put_auth_request(
        &self,
        _user: UserId,
        _key: &str,
        _req: AuthRequest,
    ) -> StoreResult<()> {
        Err(nyi("put_auth_request"))
    }
    async fn take_auth_request(
        &self,
        _user: UserId,
        _key: &str,
    ) -> StoreResult<Option<AuthRequest>> {
        Err(nyi("take_auth_request"))
    }

    // --- repo registry (M8) ---------------------------------------

    async fn list_repos(&self, _lang: LanguageCode) -> StoreResult<Vec<RepoRecord>> {
        Err(nyi("list_repos"))
    }
    async fn register_repo(
        &self,
        _lang: LanguageCode,
        _r: NewRepo,
    ) -> StoreResult<RepoId> {
        Err(nyi("register_repo"))
    }
    async fn unregister_repo(
        &self,
        _lang: LanguageCode,
        _repo: RepoId,
    ) -> StoreResult<()> {
        Err(nyi("unregister_repo"))
    }
    async fn lookup_repo(
        &self,
        _lang: LanguageCode,
        _repo: RepoId,
    ) -> StoreResult<RepoRecord> {
        Err(nyi("lookup_repo"))
    }

    // --- burrito metadata (M8) ------------------------------------

    async fn get_burrito_metadata(
        &self,
        _lang: LanguageCode,
        _repo: RepoId,
    ) -> StoreResult<BurritoMetadata> {
        Err(nyi("get_burrito_metadata"))
    }
    async fn put_burrito_metadata(
        &self,
        _lang: LanguageCode,
        _repo: RepoId,
        _m: BurritoMetadata,
    ) -> StoreResult<()> {
        Err(nyi("put_burrito_metadata"))
    }
    async fn list_ingredient_summaries(
        &self,
        _lang: LanguageCode,
        _repo: RepoId,
    ) -> StoreResult<Vec<IngredientSummary>> {
        Err(nyi("list_ingredient_summaries"))
    }

    // --- legacy repo path resolution ------------------------------

    async fn repo_workspace_path(
        &self,
        _repo_path: &std::path::Path,
    ) -> StoreResult<std::path::PathBuf> {
        // Hosted Phase 2 deployments don't have a workspace root —
        // working trees are lazy-cloned per request via
        // GitWorkspace. Endpoints that still resolve via the legacy
        // path scheme need to migrate to RepoId-keyed lookups
        // before they can run on the Supabase backend.
        Err(nyi("repo_workspace_path (legacy path scheme)"))
    }

    fn workspace_root(&self) -> &std::path::Path {
        // Same caveat as above. Returning an empty path so callers
        // that ignore the result don't crash; callers that USE it
        // will see a misbehaviour and need to migrate.
        std::path::Path::new("")
    }

    // --- multi-write atomicity (M8) -------------------------------

    async fn with_tx<'a>(
        &'a self,
        _f: Box<
            dyn for<'t> FnOnce(&'t mut (dyn Tx + 'a)) -> BoxFuture<'t, StoreResult<()>>
                + Send
                + 'a,
        >,
    ) -> StoreResult<()> {
        Err(nyi("with_tx"))
    }
}
