use crate::server::{git_dispatch, BlockingPools, LanguageLocks};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use git2::Repository;
use regex::Regex;
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;

/// *`POST /remote/delete/<repo_path>?remote_name=<...>`*
#[post("/remote/delete/<repo_path..>?<remote_name>")]
pub async fn delete_remote_from_repo(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    repo_path: PathBuf,
    remote_name: String,
) -> status::Custom<(ContentType, String)> {
    if !check_path_components(&mut repo_path.components().clone()) {
        return not_ok_bad_repo_json_response();
    }
    let repo_path_string = format!(
        "{}{}{}",
        store.workspace_root().to_string_lossy().into_owned(),
        os_slash_str(),
        &repo_path.display().to_string()
    );
    let remote_name_re = Regex::new(r"[^A-Za-z0-9_-]").unwrap();
    if remote_name_re.is_match(&remote_name) {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response("Remote name contains invalid characters".to_string()),
        );
    }
    let lang = state.default_language.clone();
    let remote_name_owned = remote_name.clone();

    let result = git_dispatch::run_locked_write(locks, pools, &lang, move || {
        let repo = Repository::open(&repo_path_string)
            .map_err(|e| format!("Could not open repo: {}", e))?;
        repo.remote_delete(&remote_name_owned)
            .map_err(|e| format!("Could not delete remote from repo: {}", e))?;
        Ok(())
    })
    .await;

    match result {
        Ok(()) => ok_ok_json_response(),
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(e.to_string()),
        ),
    }
}
