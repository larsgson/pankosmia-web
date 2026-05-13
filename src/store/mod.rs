//! Storage abstraction.
//!
//! Endpoints call into these traits instead of `std::fs::*` directly.
//! Two implementations:
//!   * `FsLanguageStore` for single-tenant FS deployments (desktop / dev).
//!   * `GitHubLanguageStore` for hosted, GitHub-backed deployments.
//!
//! Runtime selection via `STORAGE_BACKEND=fs|github` (default `fs`).

pub mod blob_store;
pub mod fs;
pub mod git_workspace;
pub mod github;
pub mod project_store;
pub mod selector;
pub mod types;

pub use blob_store::BlobStore;
pub use git_workspace::{GitWorkspace, WorkingCopy};
pub use project_store::{ProjectStore, Tx};
pub use types::{
    AppState, AuthRequest, Bcv, BlobKey, BurritoMetadata, IngredientSummary, LanguageMembership,
    NewProject, NewRepo, ProjectSummary, RepoRecord, Role, StoreError, StoreResult, TempId,
    TempUploadHandle, Typography, UserSettings,
};

/// Trait-object form of `ProjectStore` that Rocket manages as state.
/// Endpoints take `&State<SharedProjectStore>`.
pub type SharedProjectStore = std::sync::Arc<dyn project_store::ProjectStore>;
