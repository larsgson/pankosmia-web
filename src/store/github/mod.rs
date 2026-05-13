//! GitHub-backed `ProjectStore` (hosted, multi-tenant).
//!
//! On-disk layout: a per-language local clone cache, used only for
//! reads. The source of truth lives on GitHub. Writes go through the
//! GitHub App's installation token and the Contents API — no forks,
//! no per-user clones. See `internal-docs/AUTH_MODEL.md`.

pub mod audio_ref;
pub mod edit_flow;
pub mod store;

pub use audio_ref::{AudioRefConfig, AudioRefError, AUDIO_REF_CONTENT_TYPE};
pub use edit_flow::{EditFlowError, GithubEditFlow, SaveOp, SaveOutcome};
pub use store::GitHubLanguageStore;
