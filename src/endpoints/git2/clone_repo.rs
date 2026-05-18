use crate::gitea::{resolve_read_source, CuratedOrgs, ReadSource};
use crate::server::{git_dispatch, BlockingPools, LanguageLocks};
use crate::static_vars::NET_IS_ENABLED;
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, not_ok_offline_json_response,
    ok_ok_json_response,
};
use git2::{build::RepoBuilder, AutotagOption, FetchOptions, Repository};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

/// POST /clone-repo/<repo_path>?<branch>
#[post("/clone-repo/<repo_path..>?<branch>")]
pub async fn clone_repo(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    repo_path: PathBuf,
    branch: Option<String>,
) -> status::Custom<(ContentType, String)> {
    if matches!(resolve_read_source(curated, &repo_path), ReadSource::Gitea(_)) {
        return ok_ok_json_response();
    }
    if !NET_IS_ENABLED.load(Ordering::Relaxed) {
        return not_ok_offline_json_response();
    }
    let mut path_components = repo_path.components();
    if !check_path_components(&mut path_components.clone()) {
        return not_ok_bad_repo_json_response();
    }
    let source = match path_components.next().and_then(|c| c.as_os_str().to_str()) {
        Some(s) => s.to_string(),
        None => return not_ok_bad_repo_json_response(),
    };
    let org = match path_components.next().and_then(|c| c.as_os_str().to_str()) {
        Some(s) => s.to_string(),
        None => return not_ok_bad_repo_json_response(),
    };
    let mut repo = match path_components.next().and_then(|c| c.as_os_str().to_str()) {
        Some(s) => s.to_string(),
        None => return not_ok_bad_repo_json_response(),
    };
    if repo.ends_with(".git") {
        let repo_vec = repo.split('.').collect::<Vec<&str>>();
        let short_repo = &repo_vec[0..repo_vec.len() - 1];
        repo = short_repo.join("/");
    }
    let url = "https://".to_string() + &repo_path.display().to_string().replace('\\', "/");
    let local_path_str = format!(
        "{}{}{}{}{}{}{}",
        store.workspace_root().to_string_lossy().into_owned(),
        os_slash_str(),
        source,
        os_slash_str(),
        org,
        os_slash_str(),
        repo,
    );
    let lang = state.default_language.clone();
    let username = whoami::username();

    let result = git_dispatch::run_locked_write(locks, pools, &lang, move || {
        let local_path = std::path::Path::new(&local_path_str);
        let new_repo = if let Some(selected_branch) = branch {
            let mut fetch_opts = FetchOptions::new();
            fetch_opts.download_tags(AutotagOption::All);
            fetch_opts.depth(1);
            let mut builder = RepoBuilder::new();
            builder.fetch_options(fetch_opts);
            builder.branch(&selected_branch);
            builder
                .clone(&url, local_path)
                .map_err(|e| format!("could not clone branch {}: {}", selected_branch, e))?
        } else {
            Repository::clone(&url, local_path)
                .map_err(|e| format!("could not clone repo: {}", e))?
        };
        let mut config = new_repo
            .config()
            .map_err(|e| format!("repo config: {}", e))?;
        config
            .set_str("user.name", &username)
            .map_err(|e| format!("set user.name: {}", e))?;
        config
            .set_str("user.email", &format!("{}@localhost", &username))
            .map_err(|e| format!("set user.email: {}", e))?;
        Ok(())
    })
    .await;

    match result {
        Ok(()) => ok_ok_json_response(),
        Err(e) => not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(e.to_string()),
        ),
    }
}
