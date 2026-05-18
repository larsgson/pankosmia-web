//! Storage abstraction.
//!
//! Endpoints call into these traits instead of `std::fs::*` directly.
//! `GitHubLanguageStore` is the single implementation: multi-tenant,
//! GitHub-backed. User identity via the GitHub App's user-authorization
//! flow; writes via the App's installation token.

pub mod blob_store;
pub mod git_workspace;
pub mod github;
pub mod paths;
pub mod project_store;
pub mod selector;
pub mod sqlite_user_state;
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
