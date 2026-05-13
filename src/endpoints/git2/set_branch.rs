use crate::server::{git_dispatch, BlockingPools, LanguageLocks};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use git2::{build::CheckoutBuilder, Repository, StatusOptions};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;

/// *`POST /branch/<branch_ref>/<repo_path>`*
#[post("/branch/<branch_ref>/<repo_path..>")]
pub async fn set_branch(
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
        repo_path.display()
    );
    let lang = state.default_language.clone();
    let branch_full = format!("refs/heads/{}", branch_ref);

    let result = git_dispatch::run_locked_write(locks, pools, &lang, move || {
        let repo = Repository::open(&repo_path_string)
            .map_err(|e| format!("could not open repo: {}", e))?;
        let mut status_opts = StatusOptions::new();
        status_opts
            .include_untracked(true)
            .recurse_untracked_dirs(true);
        let statuses = repo
            .statuses(Some(&mut status_opts))
            .map_err(|e| format!("status check failed: {}", e))?;
        let has_changes = statuses.iter().any(|entry| {
            let s = entry.status();
            s.is_wt_modified()
                || s.is_wt_new()
                || s.is_wt_deleted()
                || s.is_index_modified()
                || s.is_index_new()
                || s.is_index_deleted()
        });
        if has_changes {
            return Err(
                "Uncommitted changes detected. Commit or stash before switching branches."
                    .to_string(),
            );
        }
        let obj = repo
            .revparse_single(&branch_full)
            .map_err(|e| format!("cannot resolve branch: {}", e))?;
        repo.checkout_tree(&obj, Some(CheckoutBuilder::new().safe()))
            .map_err(|e| format!("checkout failed: {}", e))?;
        repo.set_head(&branch_full)
            .map_err(|e| format!("set_head failed: {}", e))?;
        Ok(())
    })
    .await;

    match result {
        Ok(()) => ok_ok_json_response(),
        Err(crate::server::git_dispatch::GitOpError::Git(msg)) if msg.contains("Uncommitted") => {
            not_ok_json_response(Status::Conflict, make_bad_json_data_response(msg))
        }
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(e.to_string()),
        ),
    }
}
