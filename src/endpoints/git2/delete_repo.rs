use crate::server::{BlockingPools, LanguageLocks};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;

/// *`POST /delete/<repo_path>`*
#[post("/delete/<repo_path..>")]
pub async fn delete_repo(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    if !check_path_components(&mut repo_path.components().clone()) {
        return not_ok_bad_repo_json_response();
    }
    let path_to_delete = store.workspace_root().to_string_lossy().into_owned()
        + os_slash_str()
        + &repo_path.display().to_string();
    let lang = state.default_language.clone();

    // Not a git2 op, but recursive directory deletion is heavyweight
    // and contention-prone — dispatch to the cpu pool with a write
    // lock so concurrent deletes serialize per-language.
    let lock = locks.for_language(&lang);
    let _guard = lock.write().await;
    let join = match pools
        .run_cpu(move || std::fs::remove_dir_all(path_to_delete))
        .await
    {
        Ok(j) => j,
        Err(_) => {
            return not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response("cpu pool full".to_string()),
            );
        }
    };
    match join.await {
        Ok(Ok(())) => ok_ok_json_response(),
        Ok(Err(e)) => not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!("could not delete repo: {}", e)),
        ),
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("delete task panic: {}", e)),
        ),
    }
}
