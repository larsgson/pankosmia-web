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
use crate::store::sqlite_user_state::SqliteUserState;
use crate::store::SharedProjectStore;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

pub struct StoreBundle {
    pub store: SharedProjectStore,
    pub sqlite: Option<Arc<SqliteUserState>>,
}

pub fn build_project_store(
    workspace_root: PathBuf,
    catalog: Option<Arc<CatalogRegistry>>,
) -> StoreBundle {
    match env::var("STORAGE_BACKEND")
        .unwrap_or_else(|_| "fs".into())
        .to_ascii_lowercase()
        .as_str()
    {
        "fs" => StoreBundle {
            store: Arc::new(FsLanguageStore::new(workspace_root)),
            sqlite: None,
        },
        "github" => {
            let registry =
                catalog.expect("STORAGE_BACKEND=github requires a CatalogRegistry; pass Some(...)");
            let mut store = GitHubLanguageStore::new(workspace_root, registry);
            let sqlite = open_sqlite_if_configured().map(Arc::new);
            if let Some(ref db) = sqlite {
                store = store.with_sqlite(Arc::clone(db));
            }
            StoreBundle {
                store: Arc::new(store),
                sqlite,
            }
        }
        other => panic!("unknown STORAGE_BACKEND={}", other),
    }
}

fn open_sqlite_if_configured() -> Option<SqliteUserState> {
    let path = env::var("PANKOSMIA_SQLITE_PATH")
        .ok()
        .filter(|s| !s.is_empty())?;
    match SqliteUserState::open(std::path::Path::new(&path)) {
        Ok(db) => {
            println!("SQLite user state: {}", path);
            Some(db)
        }
        Err(e) => {
            eprintln!("WARN: SQLite user state failed to open {}: {}", path, e);
            None
        }
    }
}
