use crate::gitea::{resolve_read_source, CuratedOrgs, ReadSource};
use crate::server::{git_dispatch, BlockingPools, LanguageLocks};
use crate::static_vars::NET_IS_ENABLED;
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, not_ok_offline_json_response,
    ok_json_response,
};
use git2::{AutotagOption, FetchOptions, RemoteUpdateFlags, Repository};
use regex::Regex;
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{post, State};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

fn fast_forward(
    repo: &Repository,
    lb: &mut git2::Reference,
    rc: &git2::AnnotatedCommit,
) -> Result<(), git2::Error> {
    let name = match lb.name() {
        Some(s) => s.to_string(),
        None => String::from_utf8_lossy(lb.name_bytes()).to_string(),
    };
    let msg = format!("Fast-Forward: Setting {} to id: {}", name, rc.id());
    lb.set_target(rc.id(), &msg)?;
    repo.set_head(&name)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
    Ok(())
}

fn normal_merge(
    repo: &Repository,
    local: &git2::AnnotatedCommit,
    remote: &git2::AnnotatedCommit,
) -> Result<bool, git2::Error> {
    let local_tree = repo.find_commit(local.id())?.tree()?;
    let remote_tree = repo.find_commit(remote.id())?.tree()?;
    let ancestor = repo
        .find_commit(repo.merge_base(local.id(), remote.id())?)?
        .tree()?;
    let mut idx = repo.merge_trees(&ancestor, &local_tree, &remote_tree, None)?;
    if idx.has_conflicts() {
        repo.checkout_index(Some(&mut idx), None)?;
        return Ok(true);
    }
    let result_tree = repo.find_tree(idx.write_tree_to(repo)?)?;
    let msg = format!("Merge: {} into {}", remote.id(), local.id());
    let sig = repo.signature()?;
    let local_commit = repo.find_commit(local.id())?;
    let remote_commit = repo.find_commit(remote.id())?;
    let _merge_commit = repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        &msg,
        &result_tree,
        &[&local_commit, &remote_commit],
    )?;
    repo.checkout_head(None)?;
    Ok(false)
}

/// *`POST /pull-repo/<remote_name>/<repo_path>`*
#[post("/pull-repo/<remote_name>/<repo_path..>")]
pub async fn pull_repo(
    state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    locks: &State<LanguageLocks>,
    pools: &State<BlockingPools>,
    remote_name: &str,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    if matches!(
        resolve_read_source(curated, &repo_path),
        ReadSource::Gitea(_)
    ) {
        return ok_json_response(
            serde_json::to_string(&json!({
                "is_good": true,
                "reason": "ok",
                "merge_type": "up-to-date",
                "has_conflicts": false,
            }))
            .unwrap(),
        );
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
    let remote_transport_regex = Regex::new(r"^(https?|ssh)://|git@").unwrap();
    if remote_transport_regex.is_match(remote_name) && !NET_IS_ENABLED.load(Ordering::Relaxed) {
        return not_ok_offline_json_response();
    }
    let lang = state.default_language.clone();
    let remote_name_owned = remote_name.to_string();

    let result = git_dispatch::run_locked_write(locks, pools, &lang, move || {
        let repo = Repository::open(&repo_path_string)
            .map_err(|e| format!("could not open repo: {}", e))?;
        let mut remote = repo
            .find_remote(&remote_name_owned)
            .or_else(|_| repo.remote_anonymous(&remote_name_owned))
            .map_err(|e| format!("could not find remote: {}", e))?;
        let mut fo = FetchOptions::new();
        remote
            .download(&[] as &[&str], Some(&mut fo))
            .map_err(|e| format!("could not fetch repo: {}", e))?;
        remote
            .disconnect()
            .map_err(|e| format!("could not disconnect remote: {}", e))?;
        remote
            .update_tips(
                None,
                RemoteUpdateFlags::UPDATE_FETCHHEAD,
                AutotagOption::Unspecified,
                None,
            )
            .map_err(|e| format!("could not update tips: {}", e))?;
        let fetch_head_ref = repo
            .find_reference("FETCH_HEAD")
            .map_err(|e| format!("FETCH_HEAD: {}", e))?;
        let fetch_commit = repo
            .reference_to_annotated_commit(&fetch_head_ref)
            .map_err(|e| format!("annotated_commit: {}", e))?;
        let analysis = repo
            .merge_analysis(&[&fetch_commit])
            .map_err(|e| format!("merge_analysis: {}", e))?;
        let mut merge_type = "fast-forward";
        let mut has_conflicts = false;
        if analysis.0.is_fast_forward() {
            let head = repo.head().map_err(|e| format!("head: {}", e))?;
            let head_branch_name = head
                .name()
                .ok_or_else(|| "head has no name".to_string())?
                .to_string();
            let mut r = repo.find_reference(&head_branch_name).map_err(|e| {
                format!(
                    "could not find branch reference {}: {}",
                    head_branch_name, e
                )
            })?;
            fast_forward(&repo, &mut r, &fetch_commit)
                .map_err(|e| format!("Could not fast forward: {}", e))?;
        } else if analysis.0.is_normal() {
            merge_type = "normal";
            let head_commit = repo
                .reference_to_annotated_commit(&repo.head().map_err(|e| format!("head: {}", e))?)
                .map_err(|e| format!("annotated_commit: {}", e))?;
            has_conflicts = normal_merge(&repo, &head_commit, &fetch_commit)
                .map_err(|e| format!("Could not normal merge: {}", e))?;
        } else {
            merge_type = "up-to-date";
        }
        Ok(serde_json::to_string(&json!({
            "is_good": true,
            "reason": "ok",
            "merge_type": merge_type,
            "has_conflicts": has_conflicts,
        }))
        .unwrap_or_else(|_| "{}".to_string()))
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
