use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::burrito2::github_save::{handle_github_bulk, validate_ipath_segments};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::BulkOp;
use crate::store::sqlite_user_state::SqliteUserState;
use rocket::http::{ContentType, CookieJar};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;
use std::sync::Arc;

/// *`POST /ingredients/delete/<repo_path>?ipath=my_burrito_path`*
///
/// Typically mounted as **`/burrito/ingredients/delete/<repo_path>?ipath=my_burrito_path`**
///
/// Deletes a directory from a repo. This is an atomic multi-file
/// delete via the Git Data API (see `docs/impl/BULK_OPS.md` §3.1).
#[post("/ingredients/delete/<repo_path..>?<ipath>")]
#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)]
pub async fn post_delete_ingredients(
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
    ipath: String,
) -> status::Custom<(ContentType, String)> {
    if let Err(resp) = validate_ipath_segments(&[&ipath]) {
        return resp;
    }
    // The bulk-delete prefix is "ingredients/<ipath>" — we
    // delete every file under that subtree of the working
    // branch.
    let prefix = if ipath.is_empty() {
        "ingredients".to_string()
    } else {
        format!("ingredients/{}", ipath.trim_end_matches('/'))
    };
    let commit_message = format!("pankosmia: bulk delete ingredients/{}", ipath);
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
        BulkOp::DeleteByPrefix { prefix },
        &commit_message,
    )
    .await
}
