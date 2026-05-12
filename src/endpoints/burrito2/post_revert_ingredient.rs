use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::endpoints::burrito2::github_save::{
    handle_github_op, is_github_backend, validate_ipath_segments,
};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::{GithubEditFlow, SaveOp};
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

/// *`POST /ingredient/revert/<repo_path>?ipath=my_burrito_path`*
///
/// Typically mounted as **`/burrito/ingredient/revert/<repo_path>?ipath=my_burrito_path`**
///
/// Reverts a file from a backup file, if available.
#[post("/ingredient/revert/<repo_path..>?<ipath>")]
#[allow(clippy::too_many_arguments)]
pub async fn post_revert_ingredient(
    _state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    cookies: &CookieJar<'_>,
    edit_flow: &State<GithubEditFlow>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    locks: &State<LanguageLocks>,
    rate_limiter: &State<RateLimiter>,
    language_header: Option<LanguageHeader>,
    repo_path: PathBuf,
    ipath: String,
) -> status::Custom<(ContentType, String)> {
    if is_github_backend() {
        if let Err(resp) = validate_ipath_segments(&[&ipath]) {
            return resp;
        }
        let commit_message = format!("pankosmia: revert {}", ipath);
        return handle_github_op(
            cookies,
            edit_flow,
            app_auth,
            tokens,
            github_client,
            locks,
            rate_limiter,
            language_header,
            SaveOp::Revert { ipath: &ipath },
            &commit_message,
        )
        .await;
    }
    let path_components: Components<'_> = repo_path.components();
    let full_repo_path =
        store.workspace_root().to_string_lossy().into_owned() + os_slash_str() + &repo_path.display().to_string();
    if check_path_components(&mut path_components.clone())
        && check_path_string_components(ipath.clone())
        && std::fs::metadata(&full_repo_path).is_ok()
    {
        let destination = full_repo_path + "/ingredients/" + ipath.clone().as_str();
        let destination_backup_path = format!("{}.bak", &destination);
        if !std::path::Path::new(&destination_backup_path).exists() {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("No backup file for {}", destination_backup_path)),
            );
        }
        match std::fs::rename(&destination_backup_path, &destination) {
            Ok(_) => ok_ok_json_response(),
            Err(e) => {
                not_ok_json_response(
                    Status::InternalServerError,
                    make_bad_json_data_response(format!("Could not revert file: {}", e)),
                )
            }
        }
    } else {
        not_ok_bad_repo_json_response()
    }
}
