use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::endpoints::burrito2::github_save::{
    handle_github_op, is_github_backend, validate_ipath_segments,
};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::{GithubEditFlow, SaveOp};
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

/// *`POST /ingredient/delete/<repo_path>?ipath=my_burrito_path`*
///
/// Typically mounted as **`/burrito/ingredient/delete/<repo_path>?ipath=my_burrito_path`**
///
/// Deletes a file from a repo.
#[post("/ingredient/delete/<repo_path..>?<ipath>")]
#[allow(clippy::too_many_arguments)]
pub async fn post_delete_ingredient(
    state: &State<AppSettings>,
    cookies: &CookieJar<'_>,
    edit_flow: &State<GithubEditFlow>,
    app_auth: Option<&State<GithubAppAuth>>,
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
        let commit_message = format!("pankosmia: delete {}", ipath);
        return handle_github_op(
            cookies,
            edit_flow,
            app_auth,
            tokens,
            github_client,
            locks,
            rate_limiter,
            language_header,
            SaveOp::Delete { ipath: &ipath },
            &commit_message,
        )
        .await;
    }
    let path_components: Components<'_> = repo_path.components();
    let repo_dir = state.repo_dir.lock().expect("lock for repo dir");
    let full_repo_path =
        repo_dir.clone() + os_slash_str() + &repo_path.display().to_string();
    if check_path_components(&mut path_components.clone())
        && check_path_string_components(ipath.clone())
        && std::fs::metadata(&full_repo_path).is_ok()
    {
        let destination = full_repo_path + "/ingredients/" + ipath.clone().as_str();
        if !std::path::Path::new(&destination).exists() {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("No such file: {}", destination)),
            );
        }
        let destination_backup_path = format!("{}.bak", &destination);
        match std::fs::rename(&destination, &destination_backup_path) {
            Ok(_) => ok_ok_json_response(),
            Err(e) => {
                not_ok_json_response(
                    Status::InternalServerError,
                    make_bad_json_data_response(format!("Could not delete file: {}", e)),
                )
            }
        }
    } else {
        not_ok_bad_repo_json_response()
    }
}
