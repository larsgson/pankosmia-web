//! Supabase-Postgres `ProjectStore` implementation (M6+).
//!
//! In M6 (this milestone) every method is a stub returning
//! `StoreError::Backend("not implemented")`. Subsequent milestones
//! fill them in cluster by cluster:
//!
//!   - M7 — memberships + user settings.
//!   - M8 — repos + burrito metadata.
//!   - M9 — `BlobStore` for audio (separate trait).
//!
//! The skeleton is committed now so:
//!
//!   1. The `STORAGE_BACKEND=supabase` runtime selector compiles
//!      and rejects un-implemented methods at runtime rather than
//!      at server startup.
//!   2. A CI matrix can spin up Postgres and run the migrations
//!      against this impl, surfacing schema drift early.
//!   3. M7's first real impl method drops in alongside its sibling
//!      stubs without churn.

pub mod store;

pub use store::SupabaseLanguageStore;
