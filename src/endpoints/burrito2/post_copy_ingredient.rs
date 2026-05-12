use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::endpoints::burrito2::github_save::{
    handle_github_op, is_github_backend, validate_ipath_segments,
};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::{GithubEditFlow, SaveOp};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::burrito::destination_parent;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, check_path_string_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::{Components, PathBuf};

/// *`POST /ingredient/copy/<repo_path>?src_path=<src_path>&target_path=<target_path>&delete_src`*
///
/// Typically mounted as **`/burrito/copy/<repo_path>?src_path=<src_path>&target_path=<target_path>&delete_src`**
///
/// Copies an ingredient to a new location, optionally deleting the source.
#[post("/ingredient/copy/<repo_path..>?<src_path>&<target_path>&<delete_src>")]
#[allow(clippy::too_many_arguments)]
pub async fn copy_ingredient(
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
    src_path: String,
    target_path: String,
    delete_src: Option<bool>,
) -> status::Custom<(ContentType, String)> {
    if is_github_backend() {
        if let Err(resp) = validate_ipath_segments(&[&src_path, &target_path]) {
            return resp;
        }
        let delete_src_bool = delete_src.unwrap_or(false);
        let commit_message = format!(
            "pankosmia: copy {} → {}{}",
            src_path,
            target_path,
            if delete_src_bool { " (move)" } else { "" }
        );
        return handle_github_op(
            cookies,
            edit_flow,
            app_auth,
            tokens,
            github_client,
            locks,
            rate_limiter,
            language_header,
            SaveOp::Copy {
                src_ipath: &src_path,
                target_ipath: &target_path,
                delete_src: delete_src_bool,
            },
            &commit_message,
        )
        .await;
    }
    let path_components: Components<'_> = repo_path.components();
    if check_path_components(&mut path_components.clone())
        && check_path_string_components(src_path.clone())
        && check_path_string_components(target_path.clone())
    {
        let full_src_path = format!(
            "{}{}{}{}ingredients{}{}",
            &store.workspace_root().to_string_lossy().into_owned(),
            os_slash_str(),
            &repo_path.display().to_string(),
            os_slash_str(),
            os_slash_str(),
            &src_path
        );
        // Src ingredient must exist
        if !std::path::Path::new(&full_src_path).is_file() {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response("Source ingredient not found or not a file".to_string()),
            );
        }
        let full_target_path = format!(
            "{}{}{}{}ingredients{}{}",
            &store.workspace_root().to_string_lossy().into_owned(),
            os_slash_str(),
            &repo_path.display().to_string(),
            os_slash_str(),
            os_slash_str(),
            &target_path
        );
        // src and target must not be identical
        if full_src_path == full_target_path {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response("src and target must be different".to_string()),
            )
        }
        // Make subdirs if necessary
        let target_parent = destination_parent(full_target_path.clone());
        if !std::path::Path::new(&target_parent).exists() {
            match std::fs::create_dir_all(target_parent) {
                Ok(_) => (),
                Err(e) => {
                    return not_ok_json_response(
                        Status::InternalServerError,
                        make_bad_json_data_response(format!(
                            "Could not create target parent directories: {}",
                            e
                        )),
                    )
                }
            }
        }
        // Maybe make backup file
        let destination_backup_path = format!("{}.bak", &full_target_path);
        if std::path::Path::new(&full_target_path).exists() {
            match std::fs::rename(&full_target_path, &destination_backup_path) {
                Ok(_) => (),
                Err(e) => {
                    return not_ok_json_response(
                        Status::InternalServerError,
                        make_bad_json_data_response(format!("Could not write backup file: {}", e)),
                    )
                }
            }
        }
        // copy ingredient
        match std::fs::copy(full_src_path.clone(), full_target_path) {
            Ok(_) => {}
            Err(e) => {
                return not_ok_json_response(
                    Status::BadRequest,
                    make_bad_json_data_response(format!("could not copy ingredient: {}", e).to_string()),
                )
            }
        }
        // Maybe delete src ingredient
        match delete_src {
            Some(true) => {
                match std::fs::remove_file(full_src_path) {
                    Ok(_) => { },
                    Err(e) => return not_ok_json_response(
                        Status::BadRequest,
                        make_bad_json_data_response(format!("could not delete src ingredient: {}", e).to_string()),
                    ),
                }
            },
            _ => {}
        }
        ok_ok_json_response()
    } else {
        not_ok_bad_repo_json_response()
    }
}
