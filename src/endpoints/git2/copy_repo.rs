use crate::server::{BlockingPools, LanguageLocks};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::burrito::destination_parent;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, check_path_string_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use copy_dir::copy_dir;
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;

/// *`POST /copy/<repo_path>?target_path=...&delete_src&add_ignore`*
///
/// FS-heavy operation. Per-language write lock to avoid concurrent
/// copy + delete on the same source; CPU pool dispatch so a large
/// recursive copy doesn't block Tokio worker threads.
#[post("/copy/<repo_path..>?<target_path>&<delete_src>&<add_ignore>")]
pub async fn copy_repo(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    repo_path: PathBuf,
    target_path: String,
    delete_src: Option<bool>,
    add_ignore: Option<bool>,
) -> status::Custom<(ContentType, String)> {
    if !check_path_components(&mut repo_path.components().clone())
        || !check_path_string_components(target_path.clone())
    {
        return not_ok_bad_repo_json_response();
    }
    let workspace = store.workspace_root().to_string_lossy().into_owned();
    let full_src_path = format!(
        "{}{}{}",
        &workspace,
        os_slash_str(),
        &repo_path.display().to_string()
    );
    let full_target_path = format!("{}{}{}", &workspace, os_slash_str(), &target_path);
    if !std::path::Path::new(&full_src_path).is_dir() {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response("Source repo not found or not a directory".to_string()),
        );
    }
    if std::path::Path::new(&full_target_path).is_dir() {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response("Target repo already exists".to_string()),
        );
    }
    let app_resources_dir = state.app_resources_dir.clone();
    let lang = state.default_language.clone();
    let delete_src_b = delete_src.unwrap_or(false);
    let add_ignore_b = add_ignore.unwrap_or(false);

    let lock = locks.for_language(&lang);
    let _guard = lock.write().await;

    let join = match pools
        .run_cpu(move || -> Result<(), String> {
            let target_parent = destination_parent(full_target_path.clone());
            if !std::path::Path::new(&target_parent).exists() {
                std::fs::create_dir_all(&target_parent)
                    .map_err(|e| format!("Could not create target parent directories: {}", e))?;
            }
            copy_dir(&full_src_path, &full_target_path)
                .map_err(|e| format!("could not copy repo: {}", e))?;
            if add_ignore_b {
                let template = format!(
                    "{}{}templates{}content_templates{}gitignore.txt",
                    &app_resources_dir,
                    os_slash_str(),
                    os_slash_str(),
                    os_slash_str()
                );
                let gitignore_string = std::fs::read_to_string(&template)
                    .map_err(|e| format!("Could not load gitignore template: {}", e))?;
                let path_to_repo_gitignore =
                    format!("{}{}.gitignore", full_target_path, os_slash_str());
                std::fs::write(path_to_repo_gitignore, &gitignore_string)
                    .map_err(|e| format!("Could not write gitignore to repo: {}", e))?;
            }
            if delete_src_b {
                std::fs::remove_dir_all(&full_src_path)
                    .map_err(|e| format!("could not delete src repo: {}", e))?;
            }
            Ok(())
        })
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
        Ok(Err(msg)) => not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(msg),
        ),
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("copy task panic: {}", e)),
        ),
    }
}
