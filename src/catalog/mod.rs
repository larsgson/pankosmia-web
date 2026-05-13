//! Catalog module — the "registered languages" registry.
//!
//! On boot, the server clones (or fetches) `pankosmia-org/catalog`
//! to `<workspace_root>/.pankosmia/catalog/`, parses
//! `languages.yaml`, and materializes a `Vec<RegisteredLanguage>`
//! held behind a `parking_lot::RwLock`.
//!
//! Refresh is triggered by:
//!   * `POST /webhook/catalog` (immediate, HMAC-verified).
//!   * a periodic 15-minute timer fairing.
//!
//! See `docs/STRATEGY_GITHUB_BACKEND.md` §4 for the file format
//! and §10 for failure modes.

pub mod discovery;
pub mod registry;
pub mod schema;
pub mod sync;
pub mod webhook;

pub use registry::{CatalogRegistry, RegisteredLanguage, RegistryDiff};
pub use schema::{CatalogFile, CatalogParseError};
pub use sync::{CatalogSync, CatalogSyncError, SharedCatalogSync};
pub use webhook::{catalog_webhook, language_webhook, verify_signature, WebhookSecret};
