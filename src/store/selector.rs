//! Builds the `GitHubLanguageStore` at startup.
//!
//! Reads from a `CatalogRegistry`, clones language repos lazily,
//! writes via the GitHub App's installation token.

use crate::catalog::CatalogRegistry;
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
    catalog: Arc<CatalogRegistry>,
) -> StoreBundle {
    let mut store = GitHubLanguageStore::new(workspace_root, catalog);
    let sqlite = open_sqlite_if_configured().map(Arc::new);
    if let Some(ref db) = sqlite {
        store = store.with_sqlite(Arc::clone(db));
    }
    StoreBundle {
        store: Arc::new(store),
        sqlite,
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
