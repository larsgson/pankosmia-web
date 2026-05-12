use crate::server::{git_dispatch, BlockingPools, LanguageLocks};
use crate::static_vars::NET_IS_ENABLED;
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, home_dir_string, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, not_ok_offline_json_response,
    ok_ok_json_response,
};
use git2::{Cred, RemoteCallbacks, Repository};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::serde::json::Json;
use rocket::serde::Deserialize;
use rocket::{post, State};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

#[derive(Deserialize, Clone)]
pub struct PushForm {
    remote: String,
    cred_type: String,
    username: Option<String>,
    pass_key: Option<String>,
}

/// *`POST /push/<repo_path>`*
#[post("/push/<repo_path..>", format = "json", data = "<json_form>")]
pub async fn push_repo(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    repo_path: PathBuf,
    json_form: Json<PushForm>,
) -> status::Custom<(ContentType, String)> {
    if !NET_IS_ENABLED.load(Ordering::Relaxed) {
        return not_ok_offline_json_response();
    }
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
    let form = json_form.into_inner();

    let result = git_dispatch::run_locked_write(locks, pools, &lang, move || {
        let repo = Repository::open(&repo_path_string)
            .map_err(|e| format!("Could not open repo: {}", e))?;
        let mut remote_object = repo
            .find_remote(&form.remote)
            .map_err(|e| format!("Could not find remote {}: {}", &form.remote, e))?;
        let head = repo.head().map_err(|e| format!("head: {}", e))?;
        let head_branch_name = head
            .name()
            .ok_or_else(|| "head has no name".to_string())?
            .to_string();

        let cred_type = form.cred_type.clone();
        let pass_key = form.pass_key.clone();
        let username = form.username.clone();
        let mut remote_callbacks = RemoteCallbacks::new();
        if cred_type == "https" {
            let user = username.unwrap_or_default();
            let pass = pass_key.unwrap_or_default();
            remote_callbacks.credentials(move |_, _, _| {
                Cred::userpass_plaintext(&user, &pass)
            });
        } else {
            remote_callbacks.credentials(move |_, user_from_url, _| {
                let system_user = "git".to_string();
                let user = user_from_url.unwrap_or(&system_user);
                Cred::ssh_key(
                    user,
                    Some(std::path::Path::new(&format!(
                        "{}{}.ssh{}id_{}.pub",
                        home_dir_string(),
                        os_slash_str(),
                        os_slash_str(),
                        &cred_type
                    ))),
                    std::path::Path::new(&format!(
                        "{}{}.ssh{}id_{}",
                        home_dir_string(),
                        os_slash_str(),
                        os_slash_str(),
                        &cred_type
                    )),
                    pass_key.as_deref(),
                )
            });
        }
        let mut push_options = git2::PushOptions::new();
        push_options.remote_callbacks(remote_callbacks);
        remote_object
            .push::<&str>(&[head_branch_name.as_str()], Some(&mut push_options))
            .map_err(|e| format!("Could not push repo: {}", e))?;
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
