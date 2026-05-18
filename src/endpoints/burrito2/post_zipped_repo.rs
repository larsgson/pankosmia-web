use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::burrito2::github_save::{handle_github_bulk, read_zip_into_bulk_files};
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
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zip::ZipArchive;

/// Returns true if the zip at the given path contains a top-level
/// `metadata.json` (a minimum-signal "looks like a burrito" check).
fn check_burrito_zip_path(path: &Path) -> bool {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut archive = match ZipArchive::new(f) {
        Ok(a) => a,
        Err(_) => return false,
    };
    for i in 0..archive.len() {
        let entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if let Some(name) = entry.enclosed_name() {
            if name.to_string_lossy() == "metadata.json" && entry.is_file() {
                return true;
            }
        }
    }
    false
}

/// *`POST /zipped/<repo_path>`*
///
/// Typically mounted as **`/burrito/zipped/<repo_path>`**
///
/// Writes a new repo from a zip. The zip is parsed in-memory and
/// committed as a whole-tree replace via the Git Data API.
#[post(
    "/zipped/<repo_path..>",
    format = "multipart/form-data",
    data = "<form>"
)]
#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)]
pub async fn post_zipped_repo(
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
    mut form: Form<Upload<'_>>,
) -> status::Custom<(ContentType, String)> {
    // Persist the upload to a temp file, parse the zip, sanity-
    // check that it looks like a burrito (root metadata.json),
    // then run a whole-tree replace via the Git Data API.
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
    if !check_burrito_zip_path(tmp.path()) {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(
                "Zip does not look like a burrito (need metadata.json at root)".into(),
            ),
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
    let commit_message = format!(
        "pankosmia: replace burrito contents from upload ({} files)",
        files.len()
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
        BulkOp::ReplaceTree { files },
        &commit_message,
    )
    .await
}
