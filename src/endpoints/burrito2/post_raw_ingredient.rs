use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::endpoints::burrito2::github_save::{handle_github_op, validate_ipath_segments};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::{AudioRefConfig, GithubEditFlow, SaveOp};
use crate::store::sqlite_user_state::SqliteUserState;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::not_ok_json_response;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::serde::json::Json;
use rocket::{post, State};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// *`POST /ingredient/raw/<repo_path>?ipath=my_burrito_path&update_ingredients&no_bak`*
///
/// Typically mounted as **`/burrito/ingredient/raw/<repo_path>?ipath=my_burrito_path&update_ingredients&no_bak`**
///
/// Writes a document, where the document is provided as JSON with a 'payload' key. The ipath parameter is required. There are two optional parameters:
/// - update_ingredients to rewrite the metadata (default is false)
/// - no_bak to write bak files (default is true)
///
/// The request is authenticated via the session cookie, the language
/// is read from the `X-Language-Code` header, and the edit flow
/// forks/branches/pushes/PRs against the upstream language repo.

#[post(
    "/ingredient/raw/<repo_path..>?<ipath>&<update_ingredients>&<no_bak>",
    format = "json",
    data = "<json_form>"
)]
#[allow(irrefutable_let_patterns)]
#[allow(unused_variables)]
pub async fn post_raw_ingredient(
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
    update_ingredients: Option<String>,
    no_bak: Option<String>,
    json_form: Json<Value>,
) -> status::Custom<(ContentType, String)> {
    if let Err(resp) = validate_ipath_segments(&[&ipath]) {
        return resp;
    }
    let payload = match json_form.0.get("payload").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response("missing or non-string 'payload'".into()),
            );
        }
    };
    let bytes = payload.into_bytes();
    let commit_message = format!("pankosmia: edit {}", ipath);
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
        SaveOp::Put {
            ipath: &ipath,
            bytes: &bytes,
        },
        &commit_message,
    )
    .await
}
