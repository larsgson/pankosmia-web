//! Filesystem-backed `ProjectStore` implementation.
//!
//! Used in single-tenant deployments (desktop binary, internal LAN
//! servers, dev). The Phase 2 multi-tenant Supabase impl arrives in
//! M6+; until then this is the only implementation.
//!
//! Backwards-compat property: in FS mode, `project_role` returns
//! `Some(Role::Owner)` for **any** `(user, language)` pair. This is
//! the deliberate fallback that lets `RequireRole<L>` guards stay
//! enabled in endpoint code without breaking single-tenant
//! deployments — see `PHASE2_DESIGN.md` §3.

pub mod paths;
pub mod store;

pub use store::FsLanguageStore;
