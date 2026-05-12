//! Authentication request guards.
//!
//! Guards are **opt-in per endpoint** — taking them as parameters
//! activates auth on that route. Endpoints that don't take them
//! continue to behave as before (single-tenant, no auth required).

pub mod auth_user;
pub mod github_app;
pub mod github_client;
pub mod language_context;
pub mod oauth_flow;
pub mod session;
pub mod token_store;

pub use auth_user::{AuthError, AuthUser};
pub use github_app::{resolve_installation_id, GithubAppAuth, GithubAppError};
pub use github_client::{GithubClient, GithubError};
pub use language_context::{LanguageHeader, LanguageHeaderError};
pub use token_store::{TokenStore, TokenStoreError};
