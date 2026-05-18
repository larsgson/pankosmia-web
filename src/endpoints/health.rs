//! `GET /health` — readiness probe.
//!
//! Returns 200 with `{"status":"ok", ...}` when ready, and 503 with
//! `{"status":"degraded", reasons:[...]}` when not.

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
    let mut reasons: Vec<&'static str> = Vec::new();

    let catalog_languages = catalog.len();
    if catalog_languages == 0 {
        reasons.push("catalog has no registered languages");
    }
    let app_auth_configured = app_auth.inner().is_some();
    if !app_auth_configured {
        reasons.push("GitHub App auth not configured");
    }

    let body = serde_json::json!({
        "status": if reasons.is_empty() { "ok" } else { "degraded" },
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
