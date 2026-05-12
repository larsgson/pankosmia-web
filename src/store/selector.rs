//! Runtime backend selector for `ProjectStore`.
//!
//! `STORAGE_BACKEND=fs` (default) → `FsLanguageStore` over a local
//! workspace directory.
//!
//! `STORAGE_BACKEND=supabase` → `SupabaseLanguageStore` connecting
//! to Postgres via `DATABASE_URL`. The skeleton ships in M6;
//! method bodies fill in over M7 / M8.
//!
//! The selector is sync because `lib.rs::rocket()` is sync. The
//! Postgres connection is established via `block_on` for the
//! Supabase branch; FS deployments never hit that path.

use crate::catalog::CatalogRegistry;
use crate::store::fs::FsLanguageStore;
use crate::store::github::GitHubLanguageStore;
use crate::store::supabase::SupabaseLanguageStore;
use crate::store::SharedProjectStore;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

/// Build a `ProjectStore` for the configured backend.
///
/// `STORAGE_BACKEND` values:
///   * `fs` (default) — single-tenant FS, desktop / dev.
///   * `github` — hosted Phase 2; reads from a `CatalogRegistry`,
///     clones language repos lazily.
///   * `supabase` — vestigial; the Supabase plan was superseded by
///     the GitHub strategy. Skeleton kept in tree for reference.
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
            let registry = catalog.expect(
                "STORAGE_BACKEND=github requires a CatalogRegistry; pass Some(...)",
            );
            Arc::new(GitHubLanguageStore::new(workspace_root, registry))
        }
        "supabase" => Arc::new(build_supabase()),
        other => panic!("unknown STORAGE_BACKEND={}", other),
    }
}

fn build_supabase() -> SupabaseLanguageStore {
    let url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set when STORAGE_BACKEND=supabase");
    let max_conns = env::var("DATABASE_MAX_CONNECTIONS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(10);
    let pool = futures::executor::block_on(
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_conns)
            .connect(&url),
    )
    .expect("connect to Supabase Postgres");
    SupabaseLanguageStore::new(pool)
}
