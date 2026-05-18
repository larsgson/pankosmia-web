//! Authentication request guards.
//!
//! Guards are **opt-in per endpoint** — taking them as parameters
//! activates auth on that route. Read-only endpoints work without
//! auth since all language content is public.

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
