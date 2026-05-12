//! Authentication and authorization request guards.
//!
//! Phase 2 design contract: `docs/PHASE2_DESIGN.md` §3.
//!
//! Guards are **opt-in per endpoint** — taking them as parameters
//! activates auth on that route. Endpoints that don't take them
//! continue to behave as before (single-tenant, no auth required).
//! This mirrors the SSE-pattern from 0.15.0: same URLs, new
//! dimension layered on through the request shape, old clients see
//! no change.

pub mod auth_user;
pub mod github_app;
pub mod github_client;
pub mod jwks;
pub mod language_context;
pub mod oauth_flow;
pub mod require_role;
pub mod session;
pub mod token_store;

pub use auth_user::{AuthError, AuthUser};
pub use github_app::{resolve_installation_id, GithubAppAuth, GithubAppError};
pub use github_client::{GithubClient, GithubError};
pub use jwks::JwksCache;
pub use language_context::{
    LanguageContext, LanguageContextError, LanguageHeader, MembershipCache,
};
pub use require_role::{role_level, Editor, Owner, RequireRole, RoleError, Viewer};
pub use token_store::{TokenStore, TokenStoreError};
