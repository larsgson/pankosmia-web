use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::burrito2::github_save::{
    handle_github_bulk, read_zip_into_bulk_files, validate_ipath_segments,
};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::BulkOp;
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

/// *`POST /ingredient/zipped/<repo_path>?ipath=my_burrito_path`*
///
/// Typically mounted as **`/burrito/ingredient/zipped/<repo_path>?ipath=my_burrito_path`**
///
/// Writes files or directories provided as a zip file. The zip is
/// parsed in-memory and turned into an atomic multi-file commit via
/// the Git Data API (see `docs/impl/BULK_OPS.md` §3.3).
#[post(
    "/ingredient/zipped/<repo_path..>?<ipath>",
    format = "multipart/form-data",
    data = "<form>"
)]
#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)]
pub async fn post_zipped_ingredient(
    cookies: &CookieJar<'_>,
    catalog: &State<Arc<CatalogRegistry>>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    locks: &State<LanguageLocks>,
    rate_limiter: &State<RateLimiter>,
    sqlite: &State<Option<Arc<SqliteUserState>>>,
    language_header: Option<LanguageHeader>,
    repo_path: PathBuf,
    ipath: String,
    mut form: Form<Upload<'_>>,
) -> status::Custom<(ContentType, String)> {
    if let Err(resp) = validate_ipath_segments(&[&ipath]) {
        return resp;
    }
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
    let files = match read_zip_into_bulk_files(tmp.path()) {
        Ok(f) => f,
        Err(e) => {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("zip: {}", e)),
            );
        }
    };
    // ipath is the subdirectory under `ingredients/` where the
    // zip's contents land.
    let prefix = if ipath.is_empty() {
        "ingredients".to_string()
    } else {
        format!("ingredients/{}", ipath.trim_end_matches('/'))
    };
    let commit_message = format!(
        "pankosmia: import {} ingredients via zip into {}",
        files.len(),
        prefix
    );
    handle_github_bulk(
        cookies,
        catalog,
        app_auth,
        tokens,
        github_client,
        locks,
        rate_limiter,
        sqlite,
        language_header,
        BulkOp::UploadFiles { prefix, files },
        &commit_message,
    )
    .await
}
