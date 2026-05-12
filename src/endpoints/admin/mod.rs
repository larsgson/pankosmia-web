//! Admin / review endpoints (operates on PRs against the upstream
//! language repos).
//!
//! All endpoints require the caller to be signed in AND to have at
//! least `maintain` permission on the target language's upstream
//! repo (per GitHub's collaborator API). Writes (merge / close /
//! comment) execute under the GitHub App's installation token, not
//! the user's token — same model as the save flow.

pub mod approve;
pub mod context;
pub mod pending_prs;
pub mod pr_files;
pub mod reject;

pub use approve::approve_pr;
pub use pending_prs::list_pending_prs;
pub use pr_files::list_pr_files;
pub use reject::reject_pr;
