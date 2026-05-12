//! Storage abstraction for the Phase 2 multi-tenant transition.
//!
//! Endpoints call into these traits instead of `std::fs::*` directly.
//! The two implementations (FS for single-tenant, Supabase for hosted)
//! both ship; runtime selection via `STORAGE_BACKEND=fs|supabase`.
//!
//! See `docs/PHASE2_DESIGN.md` for the design and §11 for the
//! resolved decisions that shape the trait surface.

pub mod blob_store;
pub mod fs;
pub mod git_workspace;
pub mod github;
pub mod project_store;
pub mod selector;
pub mod supabase;
pub mod types;

pub use blob_store::BlobStore;
pub use git_workspace::{GitWorkspace, WorkingCopy};
pub use project_store::{ProjectStore, Tx};
pub use types::{
    AppState, AuthRequest, Bcv, BlobKey, BurritoMetadata, IngredientSummary,
    LanguageMembership, NewProject, NewRepo, ProjectSummary, RepoRecord, Role, StoreError,
    StoreResult, TempId, TempUploadHandle, Typography, UserSettings,
};

/// Type alias for the trait-object form of `ProjectStore` that Rocket
/// manages as state. Endpoints take `&State<SharedProjectStore>`.
///
/// The selector (`STORAGE_BACKEND=fs|supabase`) hands back one of
/// the implementations wrapped in this alias — see `M5+`.
pub type SharedProjectStore = std::sync::Arc<dyn project_store::ProjectStore>;
