//! Shared types used across the storage traits.
//!
//! Many of these are aliases for existing `crate::structs` types so we
//! don't duplicate domain models — just expose them under the
//! `crate::store` namespace.

use crate::identity::{LanguageCode, UserId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Membership level on a language. Hierarchy is `Owner > Editor >
/// Viewer`. The numeric `level()` accessor is the load-bearing one
/// for `RequireRole<L>` checks; new variants (e.g. `Admin`) can be
/// added later by inserting at a higher level — see
/// `PHASE2_DESIGN.md` §11.2.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Editor,
    Owner,
}

impl Role {
    /// Numeric level for inequality comparisons. Bigger = more
    /// privileged. The exact numbers don't matter as long as the
    /// ordering is preserved.
    pub fn level(self) -> u8 {
        match self {
            Role::Viewer => 1,
            Role::Editor => 2,
            Role::Owner => 3,
        }
    }
    pub fn is_at_least(self, other: Role) -> bool {
        self.level() >= other.level()
    }
}

/// One row from `language_memberships` (Phase 2) or one entry from
/// `_members.json` (Phase 1, FS-only).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LanguageMembership {
    pub user: UserId,
    pub language: LanguageCode,
    pub role: Role,
}

/// A summary of a language a user is a member of, for listing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub language: LanguageCode,
    pub display_name: String,
    pub role: Role,
}

/// Spec for creating a new language entry. Reserved for admin /
/// bootstrap tools; not exposed via per-user endpoints.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewProject {
    pub language: LanguageCode,
    pub display_name: String,
}

/// Per-user persisted settings.
///
/// Note: deliberately drops the historical `repo_dir` field. The
/// server owns workspace paths now; clients must not provide them.
#[derive(Clone, Serialize, Deserialize)]
pub struct UserSettings {
    pub languages: Vec<LanguageCode>,
    pub typography: Typography,
    pub gitea_endpoints: BTreeMap<String, String>,
    pub my_clients: Vec<crate::structs::Client>,
}

pub type Typography = crate::structs::Typography;
pub type Bcv = crate::structs::Bcv;

/// Per-language UI state.
#[derive(Clone, Serialize, Deserialize)]
pub struct AppState {
    pub bcv: Bcv,
}

/// Short-lived OAuth flow state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthRequest {
    pub code: String,
    pub redirect_uri: String,
    pub timestamp: std::time::SystemTime,
}

/// Spec for registering a new repo. The repo's `id` is assigned by
/// the store.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewRepo {
    pub name: String,
    pub flavor: Option<String>,
}

/// One row from `repos` (Phase 2) or one entry from `_repos.json`
/// (Phase 1).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoRecord {
    pub id: crate::identity::RepoId,
    pub name: String,
    pub flavor: Option<String>,
    pub working_path: String,
}

/// Burrito-metadata as raw JSON for now. The current
/// `crate::structs::BurritoMetadata` has interior mutexes that make it
/// awkward to pass through async traits; using `Value` until the
/// burrito-cluster endpoint refactor (M3) lets us pin a clean shape.
pub type BurritoMetadata = serde_json::Value;

/// Per-ingredient summary used for directory listings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngredientSummary {
    pub path: String,
    pub mime_type: String,
    pub size: usize,
}

/// Object-storage key (Phase 2 audio uploads). Just a string newtype
/// for clarity at call sites.
#[derive(Clone, Debug)]
pub struct BlobKey(pub String);

/// Identifier for a temporary upload, e.g. a presigned-URL flow's
/// `uploadId`. UUID-based.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TempId(pub uuid::Uuid);

/// Handle for a temp upload — currently just a path; will grow when
/// presigned URLs land in M9.
pub struct TempUploadHandle {
    pub path: std::path::PathBuf,
}

/// Errors a `ProjectStore` / `BlobStore` / `GitWorkspace` operation can
/// return. Fine-grained enough to map to HTTP status codes at the
/// endpoint layer without losing context.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("backend error: {0}")]
    Backend(String),
}

pub type StoreResult<T> = Result<T, StoreError>;
