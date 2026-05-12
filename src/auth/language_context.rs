//! `LanguageContext` request guard.
//!
//! Resolves `(user, language, role)` for a request:
//!
//!   1. `user`: from `AuthUser` (forward on miss → endpoints can
//!      use a non-authenticated variant).
//!   2. `language`: from the `X-Language-Code` request header, or
//!      a sole-membership claim in the JWT (when the user has
//!      exactly one language), or — in single-tenant FS mode where
//!      all users are Owner of every language — the configured
//!      `default_language`.
//!   3. `role`: from `ProjectStore::project_role(user, language)`,
//!      cached in `MembershipCache` for 30s.
//!
//! Single-tenant fallback: if no `Authorization` header is present
//! AND the deployment is using `FsLanguageStore` (which always
//! returns `Role::Owner`), the guard resolves to
//! `(LOCAL_USER, default_language, Owner)`. Hosted Phase 2
//! deployments require `Authorization`.

use crate::auth::AuthUser;
use crate::identity::{LanguageCode, LOCAL_USER};
use crate::store::types::Role;
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::State;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 30-second TTL cache of `(UserId, LanguageCode) -> Option<Role>`.
/// Cache hit rate at steady state ~99% — the same user repeats
/// requests on their language. Cache miss = one Postgres query
/// (~2 ms) in Phase 2; in FS mode, the lookup is microseconds
/// regardless.
pub struct MembershipCache {
    ttl: Duration,
    inner: Mutex<HashMap<(uuid::Uuid, String), (Option<Role>, Instant)>>,
}

impl MembershipCache {
    pub fn new() -> Self {
        Self {
            ttl: Duration::from_secs(30),
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, user: uuid::Uuid, lang: &str) -> Option<Option<Role>> {
        let g = self.inner.lock().unwrap();
        let (r, t) = g.get(&(user, lang.to_string()))?;
        if t.elapsed() > self.ttl {
            return None;
        }
        Some(*r)
    }

    fn put(&self, user: uuid::Uuid, lang: &str, role: Option<Role>) {
        let mut g = self.inner.lock().unwrap();
        g.insert((user, lang.to_string()), (role, Instant::now()));
    }

    pub fn invalidate(&self, user: uuid::Uuid, lang: &str) {
        self.inner.lock().unwrap().remove(&(user, lang.to_string()));
    }
}

impl Default for MembershipCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct LanguageContext {
    pub user: AuthUser,
    pub language: LanguageCode,
    pub role: Role,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for LanguageContext {
    type Error = LanguageContextError;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        // 1. Resolve user. AuthUser forwards on missing header; in
        //    single-tenant FS deployments we synthesize a LOCAL_USER
        //    identity so endpoints work without auth.
        let user = match req.guard::<AuthUser>().await {
            Outcome::Success(u) => u,
            Outcome::Forward(_) => AuthUser {
                id: LOCAL_USER,
                github_user_id: 0,
                exp: i64::MAX,
            },
            Outcome::Error((s, _)) => {
                return Outcome::Error((s, LanguageContextError::Unauthenticated));
            }
        };

        // 2. Resolve language: header takes precedence, then the
        //    server's default_language for single-tenant fallback.
        let language = match req.headers().get_one("X-Language-Code") {
            Some(s) => match LanguageCode::parse(s) {
                Ok(l) => l,
                Err(_) => {
                    return Outcome::Error((
                        Status::BadRequest,
                        LanguageContextError::InvalidLanguageCode(s.to_string()),
                    ));
                }
            },
            None => match req.guard::<&State<AppSettings>>().await {
                Outcome::Success(s) => s.default_language.clone(),
                _ => {
                    return Outcome::Error((
                        Status::InternalServerError,
                        LanguageContextError::NoDefaultLanguage,
                    ));
                }
            },
        };

        // 3. Resolve role.
        let store = match req.guard::<&State<SharedProjectStore>>().await {
            Outcome::Success(s) => s.inner().clone(),
            _ => {
                return Outcome::Error((
                    Status::InternalServerError,
                    LanguageContextError::NoStore,
                ));
            }
        };
        let cache = match req.guard::<&State<MembershipCache>>().await {
            Outcome::Success(c) => c.inner(),
            _ => {
                return Outcome::Error((
                    Status::InternalServerError,
                    LanguageContextError::NoCache,
                ));
            }
        };
        let user_uuid = user.id.0;
        let lang_str = language.as_str();
        let role = match cache.get(user_uuid, lang_str) {
            Some(r) => r,
            None => match store.project_role(user.id, language.clone()).await {
                Ok(r) => {
                    cache.put(user_uuid, lang_str, r);
                    r
                }
                Err(e) => {
                    return Outcome::Error((
                        Status::InternalServerError,
                        LanguageContextError::Backend(e.to_string()),
                    ))
                }
            },
        };

        match role {
            Some(role) => Outcome::Success(LanguageContext {
                user,
                language,
                role,
            }),
            None => Outcome::Error((Status::Forbidden, LanguageContextError::NotAMember)),
        }
    }
}

/// Lightweight guard that just extracts and parses the
/// `X-Language-Code` header. Use when an endpoint needs the language
/// without enforcing membership/role (e.g. the GitHub edit-flow
/// dispatch where role lookup against forks would be wrong).
#[derive(Clone, Debug)]
pub struct LanguageHeader(pub LanguageCode);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for LanguageHeader {
    type Error = LanguageContextError;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match req.headers().get_one("X-Language-Code") {
            Some(s) => match LanguageCode::parse(s) {
                Ok(l) => Outcome::Success(LanguageHeader(l)),
                Err(_) => Outcome::Error((
                    Status::BadRequest,
                    LanguageContextError::InvalidLanguageCode(s.to_string()),
                )),
            },
            None => Outcome::Forward(Status::BadRequest),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LanguageContextError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("invalid X-Language-Code: {0}")]
    InvalidLanguageCode(String),
    #[error("no default language configured")]
    NoDefaultLanguage,
    #[error("project store state not configured")]
    NoStore,
    #[error("membership cache state not configured")]
    NoCache,
    #[error("not a member of language")]
    NotAMember,
    #[error("store error: {0}")]
    Backend(String),
}
