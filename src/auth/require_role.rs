//! `RequireRole<L>` request guard family.
//!
//! Asserts that the resolved `LanguageContext.role` meets a
//! threshold:
//!
//! ```rust,ignore
//! #[get("/some/protected/path")]
//! async fn handler(_role: RequireRole<role_level::Editor>, ...)
//!   -> Response { ... }
//! ```
//!
//! Hierarchy: `Owner > Editor > Viewer`. A request that fails the
//! role check returns 403.
//!
//! Single-tenant FS deployment: `FsLanguageStore::project_role`
//! always returns `Some(Role::Owner)`, so every `RequireRole<L>`
//! check passes. The desktop binary keeps working unchanged.
//!
//! Forward-compat for §11.2: if a future `Admin` variant is added
//! at level 4, existing `RequireRole<Editor>` callsites continue to
//! pass for admins (admin.level() >= editor.level()).

use crate::auth::LanguageContext;
use crate::store::types::Role;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use std::marker::PhantomData;

/// Marker types for the role-level threshold. Lives in `role_level`
/// to keep the type aliases short at call sites.
pub mod role_level {
    pub trait Level: Send + Sync {
        fn required() -> super::Role;
    }
    pub struct Viewer;
    pub struct Editor;
    pub struct Owner;
    impl Level for Viewer {
        fn required() -> super::Role {
            super::Role::Viewer
        }
    }
    impl Level for Editor {
        fn required() -> super::Role {
            super::Role::Editor
        }
    }
    impl Level for Owner {
        fn required() -> super::Role {
            super::Role::Owner
        }
    }
}

pub struct RequireRole<L: role_level::Level> {
    pub ctx: LanguageContext,
    _marker: PhantomData<L>,
}

#[rocket::async_trait]
impl<'r, L: role_level::Level + 'static> FromRequest<'r> for RequireRole<L> {
    type Error = RoleError;
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let ctx = match req.guard::<LanguageContext>().await {
            Outcome::Success(c) => c,
            Outcome::Error((s, _)) => return Outcome::Error((s, RoleError::NoContext)),
            Outcome::Forward(s) => return Outcome::Forward(s),
        };
        let required = L::required();
        if ctx.role.is_at_least(required) {
            Outcome::Success(RequireRole {
                ctx,
                _marker: PhantomData,
            })
        } else {
            Outcome::Error((Status::Forbidden, RoleError::Insufficient { required }))
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RoleError {
    #[error("no language context")]
    NoContext,
    #[error("role does not meet required level: {required:?}")]
    Insufficient { required: Role },
}

// Convenience aliases — endpoints just write `_role: Viewer`.
pub type Viewer = RequireRole<role_level::Viewer>;
pub type Editor = RequireRole<role_level::Editor>;
pub type Owner = RequireRole<role_level::Owner>;
