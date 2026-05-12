use crate::server::{git_dispatch, BlockingPools, LanguageLocks};
use crate::store::SharedProjectStore;
use crate::structs::{AppSettings, GitStatusRecord};
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::os_slash_str;
use crate::utils::response::{not_ok_json_response, ok_json_response};
use git2::{Repository, StatusOptions};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};
use std::path::PathBuf;

/// *`GET /status/<repo_path>`*
///
/// Typically mounted as **`/git/status/<repo_path>`**.
///
/// Returns an array of changes to the local repo from the given
/// repo path. Read-locked and dispatched on the bounded git pool.
#[get("/status/<repo_path..>")]
pub async fn git_status(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let repo_path_string: String = store.workspace_root().to_string_lossy().into_owned()
        + os_slash_str()
        + &repo_path.display().to_string();
    let lang = state.default_language.clone();

    let result = git_dispatch::run_locked_read(locks, pools, &lang, move || {
        let repo = Repository::open(&repo_path_string)
            .map_err(|e| format!("could not open repo: {}", e))?;
        if repo.is_bare() {
            return Err("cannot get status of bare repo".into());
        }
        let mut opts = StatusOptions::new();
        opts.include_untracked(true);
        let statuses = repo
            .statuses(Some(&mut opts))
            .map_err(|e| format!("could not get repo status: {}", e))?;
        let mut status_changes = Vec::new();
        for entry in statuses
            .iter()
            .filter(|e| e.status() != git2::Status::CURRENT)
        {
            let s = entry.status();
            if s.contains(git2::Status::IGNORED) {
                continue;
            }
            let change_type = if s.contains(git2::Status::INDEX_NEW) || s.contains(git2::Status::WT_NEW) {
                "new"
            } else if s.contains(git2::Status::INDEX_MODIFIED) || s.contains(git2::Status::WT_MODIFIED) {
                "modified"
            } else if s.contains(git2::Status::INDEX_DELETED) || s.contains(git2::Status::WT_DELETED) {
                "deleted"
            } else if s.contains(git2::Status::INDEX_RENAMED) || s.contains(git2::Status::WT_RENAMED) {
                "renamed"
            } else if s.contains(git2::Status::INDEX_TYPECHANGE) || s.contains(git2::Status::WT_TYPECHANGE) {
                "type_change"
            } else {
                ""
            };
            status_changes.push(GitStatusRecord {
                path: entry.path().unwrap_or("").to_string(),
                change_type: change_type.to_string(),
            });
        }
        Ok(serde_json::to_string_pretty(&status_changes)
            .map_err(|e| format!("serialize: {}", e))?)
    })
    .await;

    match result {
        Ok(body) => ok_json_response(body),
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(e.to_string()),
        ),
    }
}
