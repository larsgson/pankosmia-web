//! `FsLanguageStore` — single-tenant FS implementation of
//! `ProjectStore`.
//!
//! Storage layout: see `crate::store::fs::paths`.
//!
//! Two intentional simplifications vs. the Phase 2 Supabase impl:
//!
//!   1. **`project_role` always returns `Some(Role::Owner)`**.
//!      Single-tenant deployments have one user with full access; this
//!      makes `RequireRole<L>` guards no-op in code without conditional
//!      logic. Hosted Phase 2 deployments swap in
//!      `SupabaseLanguageStore` which actually enforces.
//!
//!   2. **`with_tx` is a no-op wrapper.** No multi-write transactions
//!      in the FS backend; all writes are committed sequentially. The
//!      Supabase impl wraps in a real Postgres transaction.

use crate::identity::{LanguageCode, RepoId, UserId};
use crate::store::fs::paths;
use crate::store::project_store::{ProjectStore, Tx};
use crate::store::types::*;
use async_trait::async_trait;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct FsLanguageStore {
    root: PathBuf,
}

impl FsLanguageStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Default, Serialize, Deserialize)]
struct MembersFile {
    /// `BTreeMap<UserId-as-string, Role>` — string keys so the file is
    /// hand-editable for ops.
    members: BTreeMap<String, Role>,
    /// Display name for the language (e.g. "French", "Arabic").
    /// Optional because the bootstrap may not have set one yet.
    display_name: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct RepoRegistry {
    repos: BTreeMap<String, RepoRecord>,
}

fn read_json_or_default<T: Default + for<'de> Deserialize<'de>>(p: &Path) -> StoreResult<T> {
    if !p.exists() {
        return Ok(T::default());
    }
    let bytes = fs::read(p)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> StoreResult<T> {
    if !p.exists() {
        return Err(StoreError::NotFound);
    }
    let bytes = fs::read(p)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> StoreResult<()> {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(v)?;
    fs::write(p, bytes)?;
    Ok(())
}

fn default_typography() -> Typography {
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

#[async_trait]
impl ProjectStore for FsLanguageStore {
    // --- identity & membership ------------------------------------

    async fn list_user_languages(&self, _user: UserId) -> StoreResult<Vec<ProjectSummary>> {
        // FS mode: walk `.pankosmia/languages/` for known languages.
        // The implicit-Owner rule applies, so any language we know
        // about is one this user has access to.
        let mut out = Vec::new();
        let langs_dir = self.root.join(paths::RESERVED_PREFIX).join("languages");
        let entries = match fs::read_dir(&langs_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let s = name.to_string_lossy();
            let lang = match LanguageCode::parse(&s) {
                Ok(l) => l,
                Err(_) => continue,
            };
            let members: MembersFile =
                read_json_or_default(&paths::members_file(&self.root, &lang))?;
            out.push(ProjectSummary {
                language: lang,
                display_name: members.display_name.unwrap_or_default(),
                role: Role::Owner,
            });
        }
        Ok(out)
    }

    async fn project_role(&self, _user: UserId, _lang: LanguageCode) -> StoreResult<Option<Role>> {
        // Single-tenant FS: every user has Owner on every language.
        // See module docs for the rationale.
        Ok(Some(Role::Owner))
    }

    async fn create_project(&self, owner: UserId, spec: NewProject) -> StoreResult<()> {
        let dir = paths::language_dir(&self.root, &spec.language);
        fs::create_dir_all(&dir)?;
        let mut m = MembersFile::default();
        m.display_name = Some(spec.display_name);
        m.members.insert(owner.to_string(), Role::Owner);
        write_json(&paths::members_file(&self.root, &spec.language), &m)
    }

    async fn add_member(&self, lang: LanguageCode, user: UserId, role: Role) -> StoreResult<()> {
        let f = paths::members_file(&self.root, &lang);
        let mut m: MembersFile = read_json_or_default(&f)?;
        m.members.insert(user.to_string(), role);
        write_json(&f, &m)
    }

    async fn remove_member(&self, lang: LanguageCode, user: UserId) -> StoreResult<()> {
        let f = paths::members_file(&self.root, &lang);
        let mut m: MembersFile = read_json_or_default(&f)?;
        m.members.remove(&user.to_string());
        write_json(&f, &m)
    }

    // --- per-user settings ----------------------------------------

    async fn get_user_settings(&self, user: UserId) -> StoreResult<UserSettings> {
        read_json(&paths::user_settings_file(&self.root, user))
    }

    async fn put_user_settings(&self, user: UserId, s: UserSettings) -> StoreResult<()> {
        write_json(&paths::user_settings_file(&self.root, user), &s)
    }

    async fn get_languages(&self, user: UserId) -> StoreResult<Vec<LanguageCode>> {
        match self.get_user_settings(user).await {
            Ok(s) => Ok(s.languages),
            Err(StoreError::NotFound) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    async fn put_languages(&self, user: UserId, langs: Vec<LanguageCode>) -> StoreResult<()> {
        let mut s = match self.get_user_settings(user).await {
            Ok(s) => s,
            Err(StoreError::NotFound) => UserSettings {
                languages: Vec::new(),
                typography: default_typography(),
                gitea_endpoints: BTreeMap::new(),
                my_clients: Vec::new(),
            },
            Err(e) => return Err(e),
        };
        s.languages = langs;
        self.put_user_settings(user, s).await
    }

    async fn get_typography(&self, user: UserId) -> StoreResult<Typography> {
        match self.get_user_settings(user).await {
            Ok(s) => Ok(s.typography),
            Err(StoreError::NotFound) => Ok(default_typography()),
            Err(e) => Err(e),
        }
    }

    async fn put_typography(&self, user: UserId, t: Typography) -> StoreResult<()> {
        let mut s = match self.get_user_settings(user).await {
            Ok(s) => s,
            Err(StoreError::NotFound) => UserSettings {
                languages: Vec::new(),
                typography: default_typography(),
                gitea_endpoints: BTreeMap::new(),
                my_clients: Vec::new(),
            },
            Err(e) => return Err(e),
        };
        s.typography = t;
        self.put_user_settings(user, s).await
    }

    // --- per-language app state -----------------------------------

    async fn get_app_state(&self, lang: LanguageCode) -> StoreResult<AppState> {
        let f = paths::app_state_file(&self.root, &lang);
        if !f.exists() {
            return Ok(AppState { bcv: default_bcv() });
        }
        let bytes = fs::read(&f)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn put_app_state(&self, lang: LanguageCode, s: AppState) -> StoreResult<()> {
        write_json(&paths::app_state_file(&self.root, &lang), &s)
    }

    async fn get_bcv(&self, lang: LanguageCode, user: UserId) -> StoreResult<Bcv> {
        let f = paths::bcv_file(&self.root, &lang, user);
        if !f.exists() {
            return Ok(default_bcv());
        }
        let bytes = fs::read(&f)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn put_bcv(&self, lang: LanguageCode, user: UserId, bcv: Bcv) -> StoreResult<()> {
        write_json(&paths::bcv_file(&self.root, &lang, user), &bcv)
    }

    // --- gitea OAuth (per-user) -----------------------------------

    async fn get_auth_token(&self, user: UserId, key: &str) -> StoreResult<Option<String>> {
        paths::validate_segment(key)?;
        let f = paths::user_auth_tokens_file(&self.root, user);
        let m: BTreeMap<String, String> = read_json_or_default(&f)?;
        Ok(m.get(key).cloned())
    }

    async fn put_auth_token(&self, user: UserId, key: &str, code: &str) -> StoreResult<()> {
        paths::validate_segment(key)?;
        let f = paths::user_auth_tokens_file(&self.root, user);
        let mut m: BTreeMap<String, String> = read_json_or_default(&f)?;
        m.insert(key.to_string(), code.to_string());
        write_json(&f, &m)
    }

    async fn delete_auth_token(&self, user: UserId, key: &str) -> StoreResult<()> {
        paths::validate_segment(key)?;
        let f = paths::user_auth_tokens_file(&self.root, user);
        let mut m: BTreeMap<String, String> = read_json_or_default(&f)?;
        m.remove(key);
        write_json(&f, &m)
    }

    async fn put_auth_request(&self, user: UserId, key: &str, req: AuthRequest) -> StoreResult<()> {
        paths::validate_segment(key)?;
        let f = paths::user_auth_requests_file(&self.root, user);
        let mut m: BTreeMap<String, AuthRequest> = read_json_or_default(&f)?;
        m.insert(key.to_string(), req);
        write_json(&f, &m)
    }

    async fn take_auth_request(&self, user: UserId, key: &str) -> StoreResult<Option<AuthRequest>> {
        paths::validate_segment(key)?;
        let f = paths::user_auth_requests_file(&self.root, user);
        let mut m: BTreeMap<String, AuthRequest> = read_json_or_default(&f)?;
        let v = m.remove(key);
        write_json(&f, &m)?;
        Ok(v)
    }

    // --- repo registry --------------------------------------------

    async fn list_repos(&self, lang: LanguageCode) -> StoreResult<Vec<RepoRecord>> {
        let r: RepoRegistry = read_json_or_default(&paths::repo_registry_file(&self.root, &lang))?;
        Ok(r.repos.into_values().collect())
    }

    async fn register_repo(&self, lang: LanguageCode, r: NewRepo) -> StoreResult<RepoId> {
        paths::validate_segment(&r.name)?;
        let f = paths::repo_registry_file(&self.root, &lang);
        let mut reg: RepoRegistry = read_json_or_default(&f)?;
        let id = RepoId::new();
        let working_path = paths::repo_dir(&self.root, &lang, id)
            .to_string_lossy()
            .into_owned();
        let rec = RepoRecord {
            id,
            name: r.name,
            flavor: r.flavor,
            working_path,
        };
        reg.repos.insert(id.to_string(), rec);
        write_json(&f, &reg)?;
        fs::create_dir_all(paths::repo_dir(&self.root, &lang, id))?;
        Ok(id)
    }

    async fn unregister_repo(&self, lang: LanguageCode, repo: RepoId) -> StoreResult<()> {
        let f = paths::repo_registry_file(&self.root, &lang);
        let mut reg: RepoRegistry = read_json_or_default(&f)?;
        reg.repos.remove(&repo.to_string());
        write_json(&f, &reg)
    }

    async fn lookup_repo(&self, lang: LanguageCode, repo: RepoId) -> StoreResult<RepoRecord> {
        let reg: RepoRegistry =
            read_json_or_default(&paths::repo_registry_file(&self.root, &lang))?;
        reg.repos
            .get(&repo.to_string())
            .cloned()
            .ok_or(StoreError::NotFound)
    }

    // --- burrito metadata -----------------------------------------

    async fn get_burrito_metadata(
        &self,
        lang: LanguageCode,
        repo: RepoId,
    ) -> StoreResult<BurritoMetadata> {
        let p = paths::repo_dir(&self.root, &lang, repo).join("metadata.json");
        if !p.exists() {
            return Err(StoreError::NotFound);
        }
        let bytes = fs::read(&p)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn put_burrito_metadata(
        &self,
        lang: LanguageCode,
        repo: RepoId,
        m: BurritoMetadata,
    ) -> StoreResult<()> {
        let p = paths::repo_dir(&self.root, &lang, repo).join("metadata.json");
        write_json(&p, &m)
    }

    async fn list_ingredient_summaries(
        &self,
        lang: LanguageCode,
        repo: RepoId,
    ) -> StoreResult<Vec<IngredientSummary>> {
        let dir = paths::repo_dir(&self.root, &lang, repo).join("ingredients");
        let mut out = Vec::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in walkdir::WalkDir::new(&dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = match entry.path().strip_prefix(&dir) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let size = entry.metadata().map(|m| m.len() as usize).unwrap_or(0);
            // Simplistic mime-type guess from extension; a real impl
            // would call into `crate::utils::mime` once endpoints
            // start using this.
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

    async fn repo_workspace_path(
        &self,
        repo_path: &std::path::Path,
    ) -> StoreResult<std::path::PathBuf> {
        paths::legacy_repo_workspace_path(&self.root, repo_path)
    }

    fn workspace_root(&self) -> &std::path::Path {
        &self.root
    }

    // --- multi-write atomicity ------------------------------------

    async fn with_tx<'a>(
        &'a self,
        f: Box<
            dyn for<'t> FnOnce(&'t mut (dyn Tx + 'a)) -> BoxFuture<'t, StoreResult<()>> + Send + 'a,
        >,
    ) -> StoreResult<()> {
        // FS no-op: just call through. Endpoints that need real
        // atomicity must tolerate the FS impl performing each write
        // sequentially without rollback.
        let mut tx = FsTx { store: self };
        f(&mut tx).await
    }
}

struct FsTx<'a> {
    store: &'a FsLanguageStore,
}

#[async_trait]
impl<'a> Tx for FsTx<'a> {
    async fn put_app_state(&mut self, lang: LanguageCode, s: AppState) -> StoreResult<()> {
        self.store.put_app_state(lang, s).await
    }
    async fn put_burrito_metadata(
        &mut self,
        lang: LanguageCode,
        repo: RepoId,
        m: BurritoMetadata,
    ) -> StoreResult<()> {
        self.store.put_burrito_metadata(lang, repo, m).await
    }
}
