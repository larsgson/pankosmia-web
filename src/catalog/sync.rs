//! Catalog source synchronisation.
//!
//! Two deployment shapes:
//!
//! - **`PANKOSMIA_CATALOG_REPO` is set** (`owner/name`). The server
//!   clones that repo into `<workspace>/.pankosmia/catalog/` at
//!   startup, reads `languages.yaml` from there, and refreshes via
//!   `git fetch` on:
//!     - the catalog-repo webhook (`POST /webhook/catalog`)
//!     - the periodic-fetch background task (every
//!       `PANKOSMIA_PERIODIC_FETCH_INTERVAL_SECS` seconds, default
//!       15 min)
//!   Adding a language is then "open a PR on the catalog repo" with
//!   no server redeploy needed.
//!
//! - **`PANKOSMIA_CATALOG_REPO` is unset**. The server reads the
//!   yaml from `PANKOSMIA_CATALOG_PATH` (or the image-baked default
//!   at `/app/catalog/languages.yaml`). Refreshes are limited to
//!   re-reading the same file path — operators are expected to keep
//!   it fresh externally (or accept that the catalog only updates
//!   on redeploy).
//!
//! This module is the bridge between those two shapes and the
//! in-memory `CatalogRegistry`.

use crate::catalog::CatalogRegistry;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum CatalogSyncError {
    #[error("io: {0}")]
    Io(String),
    #[error("git: {0}")]
    Git(String),
    #[error("parse: {0}")]
    Parse(String),
}

/// Runtime catalog-source description. Managed as Rocket state so
/// the webhook + periodic-fetch paths can call `refresh()` without
/// re-resolving env vars on each tick.
#[derive(Clone)]
pub struct CatalogSync {
    /// Local file path to read `languages.yaml` from.
    pub file_path: PathBuf,
    /// If `Some`, the directory containing a git checkout of the
    /// catalog repo. `refresh()` runs `git fetch + reset` here
    /// before re-reading `file_path`.
    pub clone_dir: Option<PathBuf>,
    /// `<owner>/<name>` form of the catalog repo. Used only for
    /// the initial clone URL.
    pub repo_slug: Option<String>,
    /// Branch to track in the catalog repo. Defaults to `main`.
    pub branch: String,
}

impl CatalogSync {
    /// Build from env, given the workspace root for the default
    /// clone location. Mutually exclusive options:
    ///
    /// - `PANKOSMIA_CATALOG_REPO=<owner>/<name>` — git mode (preferred).
    /// - `PANKOSMIA_CATALOG_PATH=<path>` — file mode (legacy / no-repo).
    ///
    /// In git mode, the clone lives at
    /// `<workspace>/.pankosmia/catalog/` and the file path is that
    /// dir's `languages.yaml`.
    pub fn from_env(workspace_root: &std::path::Path) -> Self {
        let repo_slug = std::env::var("PANKOSMIA_CATALOG_REPO")
            .ok()
            .filter(|s| !s.is_empty());
        let branch = std::env::var("PANKOSMIA_CATALOG_BRANCH")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "main".to_string());
        match repo_slug {
            Some(slug) => {
                let clone_dir = workspace_root.join(".pankosmia").join("catalog");
                let file_path = clone_dir.join("languages.yaml");
                Self {
                    file_path,
                    clone_dir: Some(clone_dir),
                    repo_slug: Some(slug),
                    branch,
                }
            }
            None => {
                let file_path = std::env::var("PANKOSMIA_CATALOG_PATH")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/app/catalog/languages.yaml"));
                Self {
                    file_path,
                    clone_dir: None,
                    repo_slug: None,
                    branch,
                }
            }
        }
    }

    /// Perform the initial clone if `clone_dir` is set and missing.
    /// Idempotent — does nothing if the clone is already present.
    pub fn ensure_clone(&self) -> Result<(), CatalogSyncError> {
        let (clone_dir, slug) = match (&self.clone_dir, &self.repo_slug) {
            (Some(d), Some(s)) => (d.clone(), s.clone()),
            _ => return Ok(()),
        };
        if clone_dir.join(".git").is_dir() {
            return Ok(());
        }
        if let Some(parent) = clone_dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CatalogSyncError::Io(format!("mkdir {}: {}", parent.display(), e)))?;
        }
        let url = format!("https://github.com/{}.git", slug);
        git2::Repository::clone(&url, &clone_dir)
            .map_err(|e| CatalogSyncError::Git(format!("clone {}: {}", url, e)))?;
        Ok(())
    }

    /// `git fetch` + `reset --hard origin/<branch>` on the clone.
    /// No-op when `clone_dir` is `None` (file-mode deployments).
    fn pull(&self) -> Result<(), CatalogSyncError> {
        let clone_dir = match &self.clone_dir {
            Some(d) => d,
            None => return Ok(()),
        };
        let repo = git2::Repository::open(clone_dir)
            .map_err(|e| CatalogSyncError::Git(format!("open: {}", e)))?;
        let mut remote = repo
            .find_remote("origin")
            .map_err(|e| CatalogSyncError::Git(format!("find origin: {}", e)))?;
        remote
            .fetch(&[self.branch.as_str()], None, None)
            .map_err(|e| CatalogSyncError::Git(format!("fetch: {}", e)))?;
        // Reset working tree to origin/<branch>.
        let remote_ref = format!("refs/remotes/origin/{}", self.branch);
        let object = repo
            .revparse_single(&remote_ref)
            .map_err(|e| CatalogSyncError::Git(format!("revparse {}: {}", remote_ref, e)))?;
        repo.reset(&object, git2::ResetType::Hard, None)
            .map_err(|e| CatalogSyncError::Git(format!("reset: {}", e)))?;
        Ok(())
    }

    /// Pull (if git-backed) then read the yaml file and reload the
    /// in-memory registry. Returns the diff (added/removed) for
    /// logging.
    pub fn refresh(
        &self,
        catalog: &CatalogRegistry,
    ) -> Result<crate::catalog::RegistryDiff, CatalogSyncError> {
        self.pull()?;
        let yaml = std::fs::read_to_string(&self.file_path).map_err(|e| {
            CatalogSyncError::Io(format!("read {}: {}", self.file_path.display(), e))
        })?;
        catalog
            .reload_from_yaml(&yaml)
            .map_err(|e| CatalogSyncError::Parse(e.to_string()))
    }
}

/// Type alias for ergonomic Rocket-state access.
pub type SharedCatalogSync = Arc<CatalogSync>;
