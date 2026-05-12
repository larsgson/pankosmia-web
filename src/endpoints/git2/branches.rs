use crate::server::{git_dispatch, BlockingPools, LanguageLocks};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    json_payload_response, not_ok_bad_repo_json_response, not_ok_json_response,
};
use git2::Repository;
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};
use serde_json::{json, Value};
use std::path::PathBuf;

/// *`GET /branches/<repo_path>`*
///
/// Typically mounted as **`/git/branches/<repo_path>`**.
#[get("/branches/<repo_path..>")]
pub async fn list_branches_for_repo(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    repo_path: PathBuf,
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
    let lang = state.default_language.clone();

    let result = git_dispatch::run_locked_read(locks, pools, &lang, move || {
        let repo = Repository::open(&repo_path_string)
            .map_err(|e| format!("Could not open repo: {}", e))?;
        let branches = repo
            .branches(None)
            .map_err(|e| format!("Could not list branches for repo: {}", e))?;
        let head = repo.head().map_err(|e| format!("head: {}", e))?;
        let head_branch_name = head.name().unwrap_or("").to_string();
        let branch_vec: Vec<Value> = branches
            .map(|b| match b {
                Ok((branch, _)) => {
                    let bname = branch.name().ok().flatten().unwrap_or("??").to_string();
                    let is_head = head_branch_name.ends_with(&format!("/{}", bname));
                    json!({ "name": bname, "is_head": is_head })
                }
                Err(_) => json!({ "name": "???" }),
            })
            .collect();
        Ok(json!({ "branches": branch_vec }))
    })
    .await;

    match result {
        Ok(v) => json_payload_response(Status::Ok, v),
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(e.to_string()),
        ),
    }
}
