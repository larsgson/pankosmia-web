//! `GET /health` — readiness probe.
//!
//! Distinct from `/version`, which only says "the process responds".
//! `/health` confirms the server's dependencies are wired:
//!
//!   * Catalog is loaded (has at least one registered language)
//!   * In `STORAGE_BACKEND=github` mode, GitHub App auth is
//!     configured (the App's private key parsed, App ID present)
//!
//! Returns 200 with `{"status":"ok", ...}` when ready, and 503 with
//! `{"status":"degraded", reasons:[...]}` when not. Reverse proxies
//! and orchestrators can use this for traffic shifting / readiness
//! gating.

use crate::auth::GithubAppAuth;
use crate::catalog::CatalogRegistry;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};
use std::sync::Arc;

#[get("/health")]
pub fn get_health(
    catalog: &State<Arc<CatalogRegistry>>,
    app_auth: &State<Option<GithubAppAuth>>,
) -> status::Custom<(ContentType, String)> {
    let backend = std::env::var("STORAGE_BACKEND").unwrap_or_else(|_| "fs".into());
    let mut reasons: Vec<&'static str> = Vec::new();

    let catalog_languages = catalog.len();
    let needs_catalog = backend.eq_ignore_ascii_case("github");
    if needs_catalog && catalog_languages == 0 {
        reasons.push("catalog has no registered languages");
    }
    let app_auth_configured = app_auth.inner().is_some();
    if needs_catalog && !app_auth_configured {
        reasons.push("GitHub App auth not configured");
    }

    let body = serde_json::json!({
        "status": if reasons.is_empty() { "ok" } else { "degraded" },
        "backend": backend,
        "catalog_languages": catalog_languages,
        "app_auth_configured": app_auth_configured,
        "reasons": reasons,
    })
    .to_string();
    if reasons.is_empty() {
        ok_json_response(body)
    } else {
        not_ok_json_response(Status::ServiceUnavailable, body)
    }
}
