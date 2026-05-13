use crate::server::{git_dispatch, BlockingPools, LanguageLocks};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use git2::{BranchType, Repository};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;

/// *`POST /new-branch/<branch_ref>/<repo_path>`*
#[post("/new-branch/<branch_ref>/<repo_path..>")]
pub async fn create_and_set_branch(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    repo_path: PathBuf,
    branch_ref: String,
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
    let branch_ref_owned = branch_ref.clone();

    let result = git_dispatch::run_locked_write(locks, pools, &lang, move || {
        let repo = Repository::open(&repo_path_string)
            .map_err(|e| format!("could not open repo: {}", e))?;
        if repo
            .find_branch(&branch_ref_owned, BranchType::Local)
            .is_ok()
        {
            return Err(format!("Branch {} already exists", &branch_ref_owned));
        }
        let head_commit = repo
            .head()
            .and_then(|h| h.peel_to_commit())
            .map_err(|e| format!("head/peel: {}", e))?;
        repo.branch(&branch_ref_owned, &head_commit, false)
            .map_err(|e| format!("Could not create branch {}: {}", &branch_ref_owned, e))?;
        repo.set_head(&format!("refs/heads/{}", branch_ref_owned))
            .map_err(|e| format!("could not set head: {}", e))?;
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
