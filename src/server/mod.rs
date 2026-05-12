//! Server-side concurrency primitives — the load-bearing pieces from
//! `docs/SCALING.md` §3.
//!
//! These types are managed as Rocket state and shared across all
//! per-language endpoints. They impose bounds on concurrency for
//! work that, unbounded, can starve the request path.

pub mod git_dispatch;
pub mod locks;
pub mod pools;
pub mod watcher_registry;

pub use locks::LanguageLocks;
pub use pools::BlockingPools;
pub use watcher_registry::{ChangeNotice, SubscriptionHandle, WatcherRegistry};
