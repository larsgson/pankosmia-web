use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::burrito2::github_save::{
    handle_github_bulk, is_github_backend, validate_ipath_segments,
};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::BulkOp;
use crate::store::sqlite_user_state::SqliteUserState;
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, check_path_string_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::{Components, PathBuf};
use std::sync::Arc;

/// *`POST /ingredients/delete/<repo_path>?ipath=my_burrito_path`*
///
/// Typically mounted as **`/burrito/ingredients/delete/<repo_path>?ipath=my_burrito_path`**
///
/// Deletes a directory from a repo. In FS mode this is a single
/// `remove_dir_all`. In GitHub mode this is an atomic multi-file
/// delete via the Git Data API (see `docs/impl/BULK_OPS.md` §3.1).
#[post("/ingredients/delete/<repo_path..>?<ipath>")]
#[allow(clippy::too_many_arguments)]
pub async fn post_delete_ingredients(
    _state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
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
    if is_github_backend() {
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
        return handle_github_bulk(
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
        .await;
    }
    let path_components: Components<'_> = repo_path.components();
    let full_repo_path = store.workspace_root().to_string_lossy().into_owned()
        + os_slash_str()
        + &repo_path.display().to_string();
    if check_path_components(&mut path_components.clone())
        && check_path_string_components(ipath.clone())
        && std::fs::metadata(&full_repo_path).is_ok()
    {
        let destination = full_repo_path + "/ingredients/" + ipath.clone().as_str();
        if !std::path::Path::new(&destination).is_dir() {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("No such dir: {}", destination)),
            );
        }
        match std::fs::remove_dir_all(&destination) {
            Ok(_) => ok_ok_json_response(),
            Err(e) => not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("Could not delete directory: {}", e)),
            ),
        }
    } else {
        not_ok_bad_repo_json_response()
    }
}
