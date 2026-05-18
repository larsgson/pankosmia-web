use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::endpoints::burrito2::github_save::{handle_github_op, validate_ipath_segments};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::{AudioRefConfig, GithubEditFlow, SaveOp};
use crate::store::sqlite_user_state::SqliteUserState;
use rocket::http::{ContentType, CookieJar};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;
use std::sync::Arc;

/// *`POST /ingredient/copy/<repo_path>?src_path=<src_path>&target_path=<target_path>&delete_src`*
///
/// Typically mounted as **`/burrito/copy/<repo_path>?src_path=<src_path>&target_path=<target_path>&delete_src`**
///
/// Copies an ingredient to a new location, optionally deleting the source.
#[post("/ingredient/copy/<repo_path..>?<src_path>&<target_path>&<delete_src>")]
#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)]
pub async fn copy_ingredient(
    cookies: &CookieJar<'_>,
    edit_flow: &State<GithubEditFlow>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    locks: &State<LanguageLocks>,
    rate_limiter: &State<RateLimiter>,
    audio_ref_cfg: &State<AudioRefConfig>,
    sqlite: &State<Option<Arc<SqliteUserState>>>,
    language_header: Option<LanguageHeader>,
    repo_path: PathBuf,
    src_path: String,
    target_path: String,
    delete_src: Option<bool>,
) -> status::Custom<(ContentType, String)> {
    if let Err(resp) = validate_ipath_segments(&[&src_path, &target_path]) {
        return resp;
    }
    let delete_src_bool = delete_src.unwrap_or(false);
    let commit_message = format!(
        "pankosmia: copy {} → {}{}",
        src_path,
        target_path,
        if delete_src_bool { " (move)" } else { "" }
    );
    handle_github_op(
        cookies,
        edit_flow,
        app_auth,
        tokens,
        github_client,
        locks,
        rate_limiter,
        audio_ref_cfg,
        sqlite,
        language_header,
        SaveOp::Copy {
            src_ipath: &src_path,
            target_ipath: &target_path,
            delete_src: delete_src_bool,
        },
        &commit_message,
    )
    .await
}
