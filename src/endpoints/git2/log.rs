use crate::server::{git_dispatch, BlockingPools, LanguageLocks};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_json_response,
};
use git2::{ObjectType, Repository, Time};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::serde::Serialize;
use rocket::{get, State};
use std::path::PathBuf;

fn print_time(time: &Time) -> String {
    let offset = time.offset_minutes();
    let (hours, minutes) = (offset / 60, offset % 60);
    let dt = match time::OffsetDateTime::from_unix_timestamp(time.seconds()) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    let off = match time::UtcOffset::from_hms(hours as i8, minutes as i8, 0) {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    let dto = dt.to_offset(off);
    let format = time::format_description::parse("[weekday repr:short] [month repr:short] [day padding:space] [hour]:[minute]:[second] [year] [offset_hour sign:mandatory][offset_minute]")
        .unwrap();
    dto.format(&format).unwrap_or_default()
}

#[derive(Serialize)]
struct CommitJson {
    id: String,
    author: String,
    date: String,
    epoch: i64,
    message: String,
}

/// *`GET /log/<repo_path>`*
#[get("/log/<repo_path..>")]
pub async fn log_repo(
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
        let mut revwalk = repo.revwalk().map_err(|e| format!("revwalk: {}", e))?;
        revwalk
            .set_sorting(git2::Sort::TIME)
            .map_err(|e| format!("set_sorting: {}", e))?;
        let head = repo.head().map_err(|e| format!("head: {}", e))?;
        let head_branch_name = head
            .name()
            .ok_or_else(|| "head has no name".to_string())?
            .to_string();
        let rev_spec = repo
            .revparse(&head_branch_name)
            .map_err(|e| format!("revparse: {}", e))?;
        if rev_spec.mode().contains(git2::RevparseMode::SINGLE) {
            let from = rev_spec.from().ok_or_else(|| "no rev from".to_string())?;
            revwalk
                .push(from.id())
                .map_err(|e| format!("push: {}", e))?;
        } else {
            let from = rev_spec
                .from()
                .ok_or_else(|| "no rev from".to_string())?
                .id();
            let to = rev_spec.to().ok_or_else(|| "no rev to".to_string())?.id();
            revwalk.push(to).map_err(|e| format!("push to: {}", e))?;
            if rev_spec.mode().contains(git2::RevparseMode::MERGE_BASE) {
                let base = repo
                    .merge_base(from, to)
                    .map_err(|e| format!("merge_base: {}", e))?;
                let o = repo
                    .find_object(base, Some(ObjectType::Commit))
                    .map_err(|e| format!("find_object: {}", e))?;
                revwalk
                    .push(o.id())
                    .map_err(|e| format!("push base: {}", e))?;
            }
            revwalk.hide(from).map_err(|e| format!("hide: {}", e))?;
        }
        revwalk
            .push_head()
            .map_err(|e| format!("push_head: {}", e))?;
        let mut out = Vec::new();
        for rev_step in revwalk {
            let commit_id = rev_step.map_err(|e| format!("rev_step: {}", e))?;
            let commit = repo
                .find_commit(commit_id)
                .map_err(|e| format!("find_commit: {}", e))?;
            out.push(CommitJson {
                id: commit.id().to_string(),
                author: commit.author().to_string(),
                date: print_time(&commit.time()),
                epoch: commit.time().seconds(),
                message: commit.message().unwrap_or("No Message").to_string(),
            });
        }
        Ok(serde_json::to_string(&out).unwrap_or_else(|_| "[]".to_string()))
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
