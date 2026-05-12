//! `GitHubLanguageStore` — Phase 2 hosted `ProjectStore` impl.
//!
//! Layered on top of a `CatalogRegistry` (which knows which
//! languages are part of the deployment) and a local
//! workspace-root for per-language clone caches.
//!
//! Read paths (G3, this milestone):
//!   * `list_user_languages` → walk the registry; the user's role
//!     per language is determined by GitHub collaborator API
//!     (cached). For G3, role defaults to `Viewer` for any user
//!     not explicitly checked — the role-lookup glue fully wires
//!     in G6 alongside the admin panel.
//!   * `project_role` → same as above; cached for the duration of
//!     `MembershipCache`'s 30-second TTL.
//!   * `repo_workspace_path(repo_path)` → resolves to the local
//!     clone of the language repo for that path (`.pankosmia/
//!     languages/<code>/`). Falls back to `legacy_repo_workspace_path`
//!     when called with the legacy `<source>/<org>/<name>` form
//!     (e.g. from clients still using the desktop-style URLs).
//!
//! Write paths (G4–G7): not implemented in this milestone. Those
//! methods return `StoreError::Backend("not implemented")` so the
//! current endpoints don't silently no-op against the GitHub
//! backend.
//!
//! Per-user settings, BCV, and gitea OAuth tokens are NOT in
//! GitHub. Per the strategy doc §8, per-user app state (BCV,
//! typography) moves to client localStorage; gitea OAuth is
//! desktop-only. This impl returns sensible defaults for those
//! methods so M2-era endpoints continue to compile and respond.

use crate::catalog::CatalogRegistry;
use crate::identity::{LanguageCode, RepoId, UserId};
use crate::store::fs::paths;
use crate::store::project_store::{ProjectStore, Tx};
use crate::store::types::*;
use async_trait::async_trait;
use futures::future::BoxFuture;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct GitHubLanguageStore {
    workspace_root: PathBuf,
    registry: Arc<CatalogRegistry>,
}

impl GitHubLanguageStore {
    pub fn new(workspace_root: PathBuf, registry: Arc<CatalogRegistry>) -> Self {
        Self {
            workspace_root,
            registry,
        }
    }

    fn language_clone_root(&self, lang: &LanguageCode) -> PathBuf {
        self.workspace_root
            .join(".pankosmia")
            .join("languages")
            .join(lang.as_str())
    }

    /// Lazy-clone or fetch the language repo's local cache. Public
    /// for use by the catalog refresh task (G2 integration) and by
    /// the language webhook receiver (G5).
    pub async fn ensure_language_clone(
        &self,
        lang: &LanguageCode,
    ) -> StoreResult<PathBuf> {
        let lang_dir = self.language_clone_root(lang);
        let entry = self
            .registry
            .get(lang)
            .ok_or(StoreError::NotFound)?;
        let url = entry.upstream_clone_url();
        // Run git2 ops on a blocking thread; this method is a
        // surface for the integration glue, not a hot path.
        let lang_dir_for_blocking = lang_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<(), git2::Error> {
            if lang_dir_for_blocking.join(".git").is_dir() {
                let repo = git2::Repository::open(&lang_dir_for_blocking)?;
                let mut remote = repo.find_remote("origin")?;
                remote.fetch(&["main"], None, None).ok();
                Ok(())
            } else {
                std::fs::create_dir_all(&lang_dir_for_blocking).ok();
                git2::Repository::clone(&url, &lang_dir_for_blocking)?;
                Ok(())
            }
        })
        .await
        .map_err(|e| StoreError::Backend(format!("clone task panic: {}", e)))?
        .map_err(|e| StoreError::Backend(format!("clone/fetch: {}", e)))?;
        Ok(lang_dir)
    }
}

fn nyi(method: &'static str) -> StoreError {
    StoreError::Backend(format!("GitHubLanguageStore::{}: not implemented yet (G4+)", method))
}

#[async_trait]
impl ProjectStore for GitHubLanguageStore {
    // --- identity & membership ------------------------------------

    async fn list_user_languages(
        &self,
        _user: UserId,
    ) -> StoreResult<Vec<ProjectSummary>> {
        // For G3: every registered language is visible to every
        // user as `Viewer`. Real role lookup arrives in G6 via the
        // GitHub collaborators API.
        Ok(self
            .registry
            .list()
            .into_iter()
            .map(|r| ProjectSummary {
                language: r.code,
                display_name: r.display_name,
                role: Role::Viewer,
            })
            .collect())
    }

    async fn project_role(
        &self,
        _user: UserId,
        lang: LanguageCode,
    ) -> StoreResult<Option<Role>> {
        // Default: Viewer for any registered language. Real role
        // lookup (calling GitHub collaborators API per user) lands
        // in G6.
        if self.registry.contains(&lang) {
            Ok(Some(Role::Viewer))
        } else {
            Ok(None)
        }
    }

    async fn create_project(
        &self,
        _owner: UserId,
        _spec: NewProject,
    ) -> StoreResult<()> {
        Err(StoreError::Backend(
            "register a new language by opening a PR on the catalog repo \
             (https://github.com/<org>/catalog), not via this endpoint"
                .into(),
        ))
    }

    async fn add_member(
        &self,
        _lang: LanguageCode,
        _user: UserId,
        _role: Role,
    ) -> StoreResult<()> {
        Err(StoreError::Backend(
            "membership is managed via GitHub repo collaborators on the language repo, \
             not via this endpoint"
                .into(),
        ))
    }

    async fn remove_member(
        &self,
        _lang: LanguageCode,
        _user: UserId,
    ) -> StoreResult<()> {
        Err(StoreError::Backend(
            "membership is managed via GitHub repo collaborators on the language repo, \
             not via this endpoint"
                .into(),
        ))
    }

    // --- per-user settings ----------------------------------------
    //
    // Per the strategy: BCV, typography move to client localStorage.
    // Server returns defaults so existing M2 endpoints respond with
    // something sensible; clients should rely on localStorage
    // instead.

    async fn get_user_settings(&self, _user: UserId) -> StoreResult<UserSettings> {
        Err(StoreError::NotFound)
    }
    async fn put_user_settings(
        &self,
        _user: UserId,
        _s: UserSettings,
    ) -> StoreResult<()> {
        // Accept silently — clients should be using localStorage.
        Ok(())
    }
    async fn get_languages(&self, _user: UserId) -> StoreResult<Vec<LanguageCode>> {
        Ok(Vec::new())
    }
    async fn put_languages(
        &self,
        _user: UserId,
        _langs: Vec<LanguageCode>,
    ) -> StoreResult<()> {
        Ok(())
    }
    async fn get_typography(&self, _user: UserId) -> StoreResult<Typography> {
        Ok(default_typography())
    }
    async fn put_typography(&self, _user: UserId, _t: Typography) -> StoreResult<()> {
        Ok(())
    }

    // --- per-language app state ----------------------------------
    //
    // Same as above — defaults; clients use localStorage.

    async fn get_app_state(&self, _lang: LanguageCode) -> StoreResult<AppState> {
        Ok(AppState { bcv: default_bcv() })
    }
    async fn put_app_state(
        &self,
        _lang: LanguageCode,
        _s: AppState,
    ) -> StoreResult<()> {
        Ok(())
    }
    async fn get_bcv(
        &self,
        _lang: LanguageCode,
        _user: UserId,
    ) -> StoreResult<Bcv> {
        Ok(default_bcv())
    }
    async fn put_bcv(
        &self,
        _lang: LanguageCode,
        _user: UserId,
        _bcv: Bcv,
    ) -> StoreResult<()> {
        Ok(())
    }

    // --- gitea OAuth ----------------------------------------------
    //
    // Desktop-only feature; not used on hosted. Empty.

    async fn get_auth_token(
        &self,
        _user: UserId,
        _key: &str,
    ) -> StoreResult<Option<String>> {
        Ok(None)
    }
    async fn put_auth_token(
        &self,
        _user: UserId,
        _key: &str,
        _code: &str,
    ) -> StoreResult<()> {
        Ok(())
    }
    async fn delete_auth_token(&self, _user: UserId, _key: &str) -> StoreResult<()> {
        Ok(())
    }
    async fn put_auth_request(
        &self,
        _user: UserId,
        _key: &str,
        _req: AuthRequest,
    ) -> StoreResult<()> {
        Ok(())
    }
    async fn take_auth_request(
        &self,
        _user: UserId,
        _key: &str,
    ) -> StoreResult<Option<AuthRequest>> {
        Ok(None)
    }

    // --- repo registry --------------------------------------------
    //
    // Each language is a single GitHub repo. Returning a list with
    // one entry preserves the legacy "repo registry" shape; the
    // RepoId is derived from the language code so it's stable.

    async fn list_repos(&self, lang: LanguageCode) -> StoreResult<Vec<RepoRecord>> {
        let entry = self
            .registry
            .get(&lang)
            .ok_or(StoreError::NotFound)?;
        let working_path = self
            .language_clone_root(&lang)
            .to_string_lossy()
            .into_owned();
        let id_namespace = uuid::Uuid::from_u128(0xa1b2c3d4_e5f6_7890_1234_567890abcdef_u128);
        let repo_id = RepoId(uuid::Uuid::new_v5(
            &id_namespace,
            entry.repo.as_bytes(),
        ));
        Ok(vec![RepoRecord {
            id: repo_id,
            name: entry.repo.clone(),
            flavor: None,
            working_path,
        }])
    }

    async fn register_repo(
        &self,
        _lang: LanguageCode,
        _r: NewRepo,
    ) -> StoreResult<RepoId> {
        Err(nyi("register_repo (use catalog repo PR)"))
    }
    async fn unregister_repo(
        &self,
        _lang: LanguageCode,
        _repo: RepoId,
    ) -> StoreResult<()> {
        Err(nyi("unregister_repo (use catalog repo PR)"))
    }
    async fn lookup_repo(
        &self,
        lang: LanguageCode,
        repo: RepoId,
    ) -> StoreResult<RepoRecord> {
        let all = self.list_repos(lang).await?;
        all.into_iter()
            .find(|r| r.id == repo)
            .ok_or(StoreError::NotFound)
    }

    // --- burrito metadata ----------------------------------------
    //
    // Read directly from the local clone of the language repo.
    // Writes are part of the G4 edit flow (not in this milestone).

    async fn get_burrito_metadata(
        &self,
        lang: LanguageCode,
        _repo: RepoId,
    ) -> StoreResult<BurritoMetadata> {
        let path = self.language_clone_root(&lang).join("metadata.json");
        if !path.exists() {
            return Err(StoreError::NotFound);
        }
        let bytes = std::fs::read(&path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
    async fn put_burrito_metadata(
        &self,
        _lang: LanguageCode,
        _repo: RepoId,
        _m: BurritoMetadata,
    ) -> StoreResult<()> {
        Err(nyi("put_burrito_metadata (G4 edit flow)"))
    }
    async fn list_ingredient_summaries(
        &self,
        lang: LanguageCode,
        _repo: RepoId,
    ) -> StoreResult<Vec<IngredientSummary>> {
        let dir = self.language_clone_root(&lang).join("ingredients");
        let mut out = Vec::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in walkdir::WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = match entry.path().strip_prefix(&dir) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let size = entry.metadata().map(|m| m.len() as usize).unwrap_or(0);
            let mime_type = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!("application/{}", e))
                .unwrap_or_else(|| "application/octet-stream".into());
            out.push(IngredientSummary {
                path: rel,
                mime_type,
                size,
            });
        }
        Ok(out)
    }

    // --- legacy repo path resolution ------------------------------
    //
    // Hosted clients should be using language-keyed paths, but the
    // M3 endpoints still accept legacy `<source>/<org>/<name>`
    // strings. We honor them by joining the workspace root + path
    // exactly as the FS impl does.

    async fn repo_workspace_path(
        &self,
        repo_path: &Path,
    ) -> StoreResult<PathBuf> {
        paths::legacy_repo_workspace_path(&self.workspace_root, repo_path)
    }

    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    async fn prefetch_language(&self, lang: LanguageCode) -> StoreResult<()> {
        self.ensure_language_clone(&lang).await.map(|_| ())
    }

    // --- multi-write atomicity -----------------------------------

    async fn with_tx<'a>(
        &'a self,
        f: Box<
            dyn for<'t> FnOnce(&'t mut (dyn Tx + 'a)) -> BoxFuture<'t, StoreResult<()>>
                + Send
                + 'a,
        >,
    ) -> StoreResult<()> {
        // Like the FS impl: no real transactions. Future Phase-3
        // (e.g. running an audit log alongside a content commit)
        // would replace this with a proper unit of work.
        let mut tx = GitHubTx { _phantom: () };
        f(&mut tx).await
    }
}

struct GitHubTx {
    _phantom: (),
}

#[async_trait]
impl Tx for GitHubTx {
    async fn put_app_state(
        &mut self,
        _lang: LanguageCode,
        _s: AppState,
    ) -> StoreResult<()> {
        Ok(())
    }
    async fn put_burrito_metadata(
        &mut self,
        _lang: LanguageCode,
        _repo: RepoId,
        _m: BurritoMetadata,
    ) -> StoreResult<()> {
        Err(StoreError::Backend(
            "GitHubTx::put_burrito_metadata: G4 edit flow not implemented yet".into(),
        ))
    }
}

fn default_typography() -> Typography {
    use std::collections::BTreeMap;
    Typography {
        font_set: "default".into(),
        size: "14".into(),
        direction: "ltr".into(),
        features: BTreeMap::new(),
    }
}

fn default_bcv() -> Bcv {
    Bcv {
        book_code: "TIT".into(),
        chapter: 1,
        verse: 1,
    }
}
