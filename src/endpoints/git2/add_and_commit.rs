use crate::server::{BlockingPools, LanguageLocks};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use git2::Repository;
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::serde::json::Json;
use rocket::serde::Deserialize;
use rocket::{post, State};
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct AddCommitForm {
    commit_message: String,
}

/// *`POST /add-and-commit/<repo_path>`*
///
/// Typically mounted as **`/git/add-and-commit/<repo_path>`**
///
/// Adds and commits modified files in a repo.
///
/// M3.5 wiring (pilot for the write-path pattern):
///   1. Take a per-language **write lock** so concurrent commits on
///      the same language serialize at the application layer
///      (avoids the `git2` index-lock dance and lets us hold a
///      single critical section across the read-modify-write cycle).
///   2. Dispatch the actual `git2` work onto the **bounded git
///      pool** so a slow commit on a large repo can't fill Tokio's
///      default blocking pool and starve the request path.
///
/// In single-tenant FS mode the language lock is taken on
/// `default_language` (every request resolves there until M5's
/// `LanguageContext` plugs in real values).
#[post("/add-and-commit/<repo_path..>", format = "json", data = "<json_form>")]
pub async fn add_and_commit(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    repo_path: PathBuf,
    json_form: Json<AddCommitForm>,
) -> status::Custom<(ContentType, String)> {
    let path_components = repo_path.components();
    if !check_path_components(&mut path_components.clone()) {
        return not_ok_bad_repo_json_response();
    }
    let repo_path_string = format!(
        "{}{}{}",
        store.workspace_root().to_string_lossy().into_owned(),
        os_slash_str(),
        &repo_path.display().to_string()
    );
    let commit_message = json_form.commit_message.clone();

    // Per-language write lock. Held for the full read-modify-write
    // cycle below; concurrent commits on the same language wait
    // here. Concurrent commits on different languages don't.
    let lock = locks.for_language(&state.default_language);
    let _guard = lock.write().await;

    // Dispatch the blocking git2 work onto the bounded git pool.
    // The closure owns its inputs so the future can be `'static`.
    let result = pools
        .run_git(move || -> Result<(), String> {
            let repo =
                Repository::open(&repo_path_string).map_err(|e| format!("open repo: {}", e))?;
            let mut index = repo.index().map_err(|e| format!("repo index: {}", e))?;
            index
                .add_all(&["."], git2::IndexAddOption::DEFAULT, None)
                .map_err(|e| format!("add_all: {}", e))?;
            index.write().map_err(|e| format!("index write: {}", e))?;
            let oid = index
                .write_tree()
                .map_err(|e| format!("write_tree: {}", e))?;
            let signature = repo.signature().map_err(|e| format!("signature: {}", e))?;
            let parent_commit = repo
                .head()
                .and_then(|h| h.peel_to_commit())
                .map_err(|e| format!("head/peel: {}", e))?;
            let tree = repo
                .find_tree(oid)
                .map_err(|e| format!("find_tree: {}", e))?;
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                &commit_message,
                &tree,
                &[&parent_commit],
            )
            .map_err(|e| format!("commit: {}", e))?;
            Ok(())
        })
        .await;

    let join = match result {
        Ok(j) => j,
        Err(e) => {
            return not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response(format!("git pool: {}", e)),
            );
        }
    };
    match join.await {
        Ok(Ok(())) => ok_ok_json_response(),
        Ok(Err(msg)) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(msg),
        ),
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("git task panicked: {}", e)),
        ),
    }
}
