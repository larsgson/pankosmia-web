use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::endpoints::burrito2::github_save::{handle_github_op, validate_ipath_segments};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::{AudioRefConfig, GithubEditFlow, SaveOp};
use crate::store::sqlite_user_state::SqliteUserState;
use crate::structs::Upload;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::not_ok_json_response;
use rocket::form::Form;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::{post, State};
use std::path::PathBuf;
use std::sync::Arc;

/// *`POST /ingredient/bytes/<repo_path>?ipath=my_burrito_path`*
///
/// Typically mounted as **`/burrito/ingredient/bytes/<repo_path>?ipath=my_burrito_path`**
///
/// Writes a document, where the document is provided as a file upload.
#[post(
    "/ingredient/bytes/<repo_path..>?<ipath>",
    format = "multipart/form-data",
    data = "<form>"
)]
#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)]
pub async fn post_bytes_ingredient(
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
    mut form: Form<Upload<'_>>,
) -> status::Custom<(ContentType, String)> {
    if let Err(resp) = validate_ipath_segments(&[&ipath]) {
        return resp;
    }
    // Move uploaded file to a temp path, then read its bytes for
    // the Contents API. (Streaming directly would need a larger
    // refactor; multipart-uploaded files are bounded by the
    // Rocket form-data limit.)
    let tmp = match tempfile::NamedTempFile::new() {
        Ok(t) => t,
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("tempfile: {}", e)),
            );
        }
    };
    if let Err(e) = form.file.persist_to(tmp.path()).await {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("persist upload: {}", e)),
        );
    }
    let bytes = match std::fs::read(tmp.path()) {
        Ok(b) => b,
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("read upload: {}", e)),
            );
        }
    };
    let commit_message = format!("pankosmia: upload {}", ipath);
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
