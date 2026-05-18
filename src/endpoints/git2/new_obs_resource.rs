use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::endpoints::burrito2::github_save::handle_github_bulk;
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::{BulkFile, BulkOp};
use crate::store::sqlite_user_state::SqliteUserState;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::os_slash_str;
use crate::utils::response::not_ok_json_response;
use crate::utils::time::utc_now_timestamp_string;
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::serde::json::Json;
use rocket::serde::Deserialize;
use rocket::{post, State};
use serde_json::json;
use std::sync::Arc;
use walkdir::WalkDir;

#[derive(Deserialize)]
pub struct NewObsContentForm {
    pub content_name: String,
    pub content_abbr: String,
    pub content_language_code: String,
    pub branch_name: Option<String>,
}

#[post("/new-obs-resource", format = "json", data = "<json_form>")]
pub async fn new_obs_resource_repo(
    state: &State<AppSettings>,
    catalog: &State<Arc<CatalogRegistry>>,
    app_auth: &State<Option<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    locks: &State<LanguageLocks>,
    rate_limiter: &State<RateLimiter>,
    sqlite: &State<Option<Arc<SqliteUserState>>>,
    language_header: Option<LanguageHeader>,
    cookies: &CookieJar<'_>,
    json_form: Json<NewObsContentForm>,
) -> status::Custom<(ContentType, String)> {
    let template_dir = format!(
        "{}templates{}content_templates{}text_stories",
        &state.app_resources_dir,
        os_slash_str(),
        os_slash_str(),
    );
    let metadata_template_path = format!("{}{}metadata.json", &template_dir, os_slash_str());
    if !std::path::Path::new(&metadata_template_path).is_file() {
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!(
                "Metadata template {} not found",
                metadata_template_path
            )),
        );
    }

    let now_time = utc_now_timestamp_string();
    let language_tag = &json_form.content_language_code;
    let language_name = &json_form.content_language_code;
    let language_json = json!({
        "tag": language_tag,
        "name": {
            "en": language_name,
        }
    });

    let mut metadata_string = match std::fs::read_to_string(&metadata_template_path) {
        Ok(v) => v,
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("Could not load metadata template: {}", e)),
            );
        }
    };
    metadata_string = metadata_string
        .replace("%%ABBR%%", &json_form.content_abbr)
        .replace("%%CONTENT_NAME%%", &json_form.content_name)
        .replace("%%CREATED_TIMESTAMP%%", &now_time)
        .replace(
            "%%LANGUAGE%%",
            &serde_json::to_string(&language_json).unwrap_or_default(),
        );

    let mut files: Vec<BulkFile> = Vec::new();

    files.push(BulkFile {
        path: "metadata.json".to_string(),
        bytes: metadata_string.into_bytes(),
    });

    let ingredients_dir = format!("{}{}ingredients", &template_dir, os_slash_str());
    for entry in WalkDir::new(&ingredients_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let full_path = entry.path();
        let rel_path = match full_path.strip_prefix(&template_dir) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let bytes = match std::fs::read(full_path) {
            Ok(b) => b,
            Err(e) => {
                return not_ok_json_response(
                    Status::InternalServerError,
                    make_bad_json_data_response(format!(
                        "Could not read template file {}: {}",
                        rel_path, e
                    )),
                );
            }
        };
        files.push(BulkFile {
            path: rel_path,
            bytes,
        });
    }

    let op = BulkOp::UploadFiles {
        prefix: String::new(),
        files,
    };

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
        op,
        &format!("New OBS resource: {}", json_form.content_abbr),
    )
    .await
}
