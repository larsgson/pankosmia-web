//! `AuthUser` request guard — session-cookie based.
//!
//! Replaces the v0.15.x JWT-based auth with a session cookie that
//! identifies the user by their GitHub user-id. The OAuth token
//! itself is never sent to the browser — the server looks it up
//! via `TokenStore` whenever it needs to call GitHub.
//!
//! Forward semantics: when there is no session cookie, this guard
//! `Outcome::Forward`s. Optional-auth routes (e.g. `/version`) work
//! without sign-in. Routes that take `LanguageContext` get a
//! synthesized `LOCAL_USER` if no real session exists, preserving
//! single-tenant FS-mode behavior. Hosted deployments add a fairing
//! that requires a real session for protected routes.

use crate::auth::session::read_session;
use crate::identity::UserId;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};

#[derive(Clone, Copy, Debug)]
pub struct AuthUser {
    pub id: UserId,
    pub github_user_id: i64,
    /// Compatibility field: previous JWT-based AuthUser exposed an
    /// `exp` timestamp used by the SSE close-at-exp behavior. With
    /// session cookies there is no per-token exp; sessions live
    /// until logout or revocation. Keep the field with a sentinel
    /// value so existing call sites compile; SSE close-at-exp is
    /// rendered moot under session-cookie auth.
    pub exp: i64,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthUser {
    type Error = AuthError;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let cookies = req.cookies();
        match read_session(cookies) {
            Some(github_user_id) => Outcome::Success(AuthUser {
                id: UserId::from_github_id(github_user_id),
                github_user_id,
                exp: i64::MAX,
            }),
            None => Outcome::Forward(Status::Unauthorized),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing session cookie")]
    Missing,
    #[error("session decode error")]
    Decode,
}
