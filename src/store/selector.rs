//! Runtime backend selector for `ProjectStore`.
//!
//! `STORAGE_BACKEND=fs` (default) → `FsLanguageStore` over a local
//! workspace directory.
//!
//! `STORAGE_BACKEND=github` → `GitHubLanguageStore`. Reads from a
//! `CatalogRegistry`, clones language repos lazily, writes via the
//! GitHub App's installation token (see `docs/AUTH_MODEL.md`).

use crate::catalog::CatalogRegistry;
use crate::store::fs::FsLanguageStore;
use crate::store::github::GitHubLanguageStore;
use crate::store::SharedProjectStore;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

pub fn build_project_store(
    workspace_root: PathBuf,
    catalog: Option<Arc<CatalogRegistry>>,
) -> SharedProjectStore {
    match env::var("STORAGE_BACKEND")
        .unwrap_or_else(|_| "fs".into())
        .to_ascii_lowercase()
        .as_str()
    {
        "fs" => Arc::new(FsLanguageStore::new(workspace_root)),
        "github" => {
            let registry =
                catalog.expect("STORAGE_BACKEND=github requires a CatalogRegistry; pass Some(...)");
            Arc::new(GitHubLanguageStore::new(workspace_root, registry))
        }
        other => panic!("unknown STORAGE_BACKEND={}", other),
    }
}
