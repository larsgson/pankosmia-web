use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::burrito2::github_save::handle_github_bulk;
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::BulkOp;
use crate::store::sqlite_user_state::SqliteUserState;
use crate::structs::AppSettings;
use rocket::http::{ContentType, CookieJar};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;
use std::sync::Arc;

/// *`POST /metadata/remake-ingredients/<repo_path>`*
///
/// Typically mounted as **`/burrito/metadata/remake-ingredients/<repo_path>`**
///
/// Remakes the ingredients section of the metadata for a repo.

#[post("/metadata/remake-ingredients/<repo_path..>")]
#[allow(irrefutable_let_patterns)]
#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)]
pub async fn remake_ingredients_metadata(
    state: &State<AppSettings>,
    cookies: &CookieJar<'_>,
    catalog: &State<Arc<CatalogRegistry>>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    locks: &State<LanguageLocks>,
    rate_limiter: &State<RateLimiter>,
    sqlite: &State<Option<Arc<SqliteUserState>>>,
    language_header: Option<LanguageHeader>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    handle_github_bulk(
        cookies,
        catalog,
        app_auth,
        tokens,
        github_client,
        locks,
        rate_limiter,
        sqlite,
        language_header,
        BulkOp::RegenerateMetadata {
            app_resources_dir: state.app_resources_dir.clone(),
        },
        "pankosmia: regenerate metadata.json ingredients",
    )
    .await
}
