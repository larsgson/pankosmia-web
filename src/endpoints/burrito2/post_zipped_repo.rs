use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::burrito2::github_save::{
    handle_github_bulk, is_github_backend, read_zip_into_bulk_files,
};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::BulkOp;
use crate::store::sqlite_user_state::SqliteUserState;
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use crate::utils::zip::unpack_zip_file;
use rocket::form::{Form, FromForm};
use rocket::fs::TempFile;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::{post, State};
use std::fs::File;
use std::path::{Components, Path, PathBuf};
use std::sync::Arc;
use tempfile::NamedTempFile;
use zip::ZipArchive;

#[derive(FromForm)]
pub struct Upload<'f> {
    file: TempFile<'f>,
}

/// Returns true if the zip at the given path contains a top-level
/// `metadata.json` (a minimum-signal "looks like a burrito" check).
/// Sibling of `check_burrito_zip` but operates on `&Path` so we
/// don't need a `NamedTempFile` handle.
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

/// Returns true if the zip looks a bit like a burrito
fn check_burrito_zip(path: &NamedTempFile) -> bool {
    let zip_file = File::open(path).expect("open zip archive file to check");
    let mut archive = ZipArchive::new(zip_file).expect("new archive to check");
    // Iterate over archive files, looking for metadata and ingredients
    let mut metadata_found = false;
    let mut ingredients_found = false;
    for i in 0..archive.len() {
        let file = archive.by_index(i).expect("file from zip to check");
        let out_path = match file.enclosed_name() {
            Some(p) => p,
            None => continue,
        };
        let out_path_string = format!("{:?}", out_path);
        if file.is_file() {
            if out_path_string == "\"metadata.json\"" {
                metadata_found = true;
            }
        } else {
            if out_path_string == "\"ingredients/\"" {
                ingredients_found = true;
            }
        }
    }
    metadata_found && ingredients_found
}

/// *`POST /zipped/<repo_path>`*
///
/// Typically mounted as **`/burrito/zipped/<repo_path>`**
///
/// Writes a new repo from a zip. The path must start with _local_/_sideloaded_
#[post(
    "/zipped/<repo_path..>",
    format = "multipart/form-data",
    data = "<form>"
)]
#[allow(clippy::too_many_arguments)]
pub async fn post_zipped_repo(
    _state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
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
    if is_github_backend() {
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
        return handle_github_bulk(
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
        .await;
    }
    let path_components: Components<'_> = repo_path.components();
    let full_repo_path = format!(
        "{}{}{}",
        store.workspace_root().to_string_lossy().into_owned(),
        os_slash_str(),
        &repo_path.display().to_string()
    );
    if check_path_components(&mut path_components.clone()) {
        let mut path_n = 0;
        for path_component in path_components {
            let path_string = path_component
                .clone()
                .as_os_str()
                .to_str()
                .unwrap()
                .to_string();
            if (path_n == 0) && (path_string != "_local_".to_string()) {
                return not_ok_json_response(
                    Status::BadRequest,
                    make_bad_json_data_response(format!(
                        "First repo path component must be '_local_' not '{}'",
                        &path_string
                    )),
                );
            }
            if (path_n == 1) && (path_string != "_sideloaded_".to_string()) {
                return not_ok_json_response(
                    Status::BadRequest,
                    make_bad_json_data_response(format!(
                        "Second repo path component must be '_sideloaded_' not '{}'",
                        &path_string
                    )),
                );
            }
            path_n += 1;
        }
        if Path::new(&full_repo_path).exists() {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response(format!("Repo already exists")),
            );
        }
        // Copy upload to temp file we manage
        let file_path = NamedTempFile::new().expect("tempfile");
        form.file.move_copy_to(&file_path).await.expect("copy zip");

        // Check burrito
        if !check_burrito_zip(&file_path) {
            return not_ok_json_response(
                Status::BadRequest,
                make_bad_json_data_response("Zip does not look like a burrito".to_string()),
            );
        }

        match std::fs::create_dir_all(&full_repo_path) {
            Ok(_) => (),
            Err(e) => {
                return not_ok_json_response(
                    Status::InternalServerError,
                    make_bad_json_data_response(format!("Could not create repo dir: {}", e)),
                )
            }
        }

        // Unpack zip
        match unpack_zip_file(file_path, full_repo_path, None).await {
            Ok(_) => ok_ok_json_response(),
            Err(e) => not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("Could not write zip archive: {}", e)),
            ),
        }
    } else {
        not_ok_bad_repo_json_response()
    }
}
