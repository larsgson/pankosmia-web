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

/// *`POST /ingredient/delete/<repo_path>?ipath=my_burrito_path`*
///
/// Typically mounted as **`/burrito/ingredient/delete/<repo_path>?ipath=my_burrito_path`**
///
/// Deletes a file from a repo.
#[post("/ingredient/delete/<repo_path..>?<ipath>")]
#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)]
pub async fn post_delete_ingredient(
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
    ipath: String,
) -> status::Custom<(ContentType, String)> {
    if let Err(resp) = validate_ipath_segments(&[&ipath]) {
        return resp;
    }
    let commit_message = format!("pankosmia: delete {}", ipath);
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
        SaveOp::Delete { ipath: &ipath },
        &commit_message,
    )
    .await
}
