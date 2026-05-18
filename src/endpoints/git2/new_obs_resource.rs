use crate::auth::session::read_session;
use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::catalog::CatalogRegistry;
use crate::identity::LanguageCode;
use crate::server::{LanguageLocks, RateLimitError, RateLimiter};
use crate::store::github::{apply_bulk_op, BulkFile, BulkOp};
use crate::store::sqlite_user_state::SqliteUserState;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::os_slash_str;
use crate::utils::response::{not_ok_json_response, ok_json_response};
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
    println!(
        "new-obs-resource: called with abbr={:?} lang={:?} name={:?}",
        json_form.content_abbr, json_form.content_language_code, json_form.content_name
    );
    let github_user_id = match read_session(cookies) {
        Some(id) => {
            println!("new-obs-resource: session user_id={}", id);
            id
        }
        None => {
            println!("new-obs-resource: FAIL no session");
            return not_ok_json_response(
                Status::Unauthorized,
                make_bad_json_data_response("not signed in".into()),
            );
        }
    };
    if let Err(RateLimitError::Exceeded(retry_after)) = rate_limiter.check(github_user_id) {
        println!("new-obs-resource: FAIL rate limited");
        return not_ok_json_response(
            Status::TooManyRequests,
            make_bad_json_data_response(format!("rate limit exceeded; retry in {}s", retry_after)),
        );
    }
    let app_auth = match app_auth.inner().as_ref() {
        Some(a) => {
            println!("new-obs-resource: app auth present");
            a
        }
        None => {
            println!("new-obs-resource: FAIL no app auth");
            return not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response(
                    "GitHub App auth not configured (GITHUB_APP_ID unset?)".into(),
                ),
            );
        }
    };
    let token = match tokens.load(github_user_id) {
        Ok(Some(t)) => {
            println!("new-obs-resource: token loaded");
            t
        }
        Ok(None) => {
            println!("new-obs-resource: FAIL no token stored");
            return not_ok_json_response(
                Status::Unauthorized,
                make_bad_json_data_response("no stored token; please sign in again".into()),
            );
        }
        Err(e) => {
            println!("new-obs-resource: FAIL token store error: {}", e);
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("token store: {}", e)),
            );
        }
    };
    let user = match github_client.get_user(&token).await {
        Ok(u) => {
            println!("new-obs-resource: github user={}", u.login);
            u
        }
        Err(e) => {
            println!("new-obs-resource: FAIL github /user: {}", e);
            return not_ok_json_response(
                Status::BadGateway,
                make_bad_json_data_response(format!("github /user: {}", e)),
            );
        }
    };

    let lang = if let Some(LanguageHeader(l)) = &language_header {
        println!("new-obs-resource: lang from header: {}", l);
        l.clone()
    } else {
        match LanguageCode::parse(&json_form.content_language_code) {
            Ok(l) => {
                println!("new-obs-resource: lang from form: {}", l);
                l
            }
            Err(e) => {
                println!("new-obs-resource: FAIL invalid language code: {}", e);
                return not_ok_json_response(
                    Status::BadRequest,
                    make_bad_json_data_response(format!("invalid language code: {}", e)),
                );
            }
        }
    };
    let _ = sqlite;

    let template_dir = format!(
        "{}templates{}content_templates{}text_stories",
        &state.app_resources_dir,
        os_slash_str(),
        os_slash_str(),
    );
    let metadata_template_path = format!("{}{}metadata.json", &template_dir, os_slash_str());
    println!("new-obs-resource: template_dir={}", template_dir);
    if !std::path::Path::new(&metadata_template_path).is_file() {
        println!(
            "new-obs-resource: FAIL metadata template not found: {}",
            metadata_template_path
        );
        return not_ok_json_response(
            Status::BadRequest,
            make_bad_json_data_response(format!(
                "Metadata template {} not found",
                metadata_template_path
            )),
        );
    }

    let now_time = utc_now_timestamp_string();
    let language_json = json!({
        "tag": &json_form.content_language_code,
        "name": {
            "en": &json_form.content_language_code,
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
    println!("new-obs-resource: metadata.json prepared");

    let ingredients_dir = format!("{}{}ingredients", &template_dir, os_slash_str());
    println!(
        "new-obs-resource: scanning ingredients at {}",
        ingredients_dir
    );
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

    println!(
        "new-obs-resource: collected {} files, dispatching bulk upload for lang={}",
        files.len(),
        lang
    );

    let op = BulkOp::UploadFiles {
        prefix: String::new(),
        files,
    };

    match apply_bulk_op(
        catalog.inner(),
        &user.login,
        github_user_id,
        lang,
        op,
        &format!("New OBS resource: {}", json_form.content_abbr),
        github_client.inner(),
        app_auth,
        locks.inner(),
    )
    .await
    {
        Ok(outcome) => {
            println!(
                "new-obs-resource: SUCCESS pr_url={} branch={}",
                outcome.pr_url, outcome.branch
            );
            let body = json!({
                "is_good": true,
                "status": outcome.status,
                "branch": outcome.branch,
                "pr_url": outcome.pr_url,
                "pr_number": outcome.pr_number,
            });
            ok_json_response(body.to_string())
        }
        Err(e) => {
            println!("new-obs-resource: FAIL bulk op: {}", e);
            not_ok_json_response(
                Status::BadGateway,
                make_bad_json_data_response(format!("github: {}", e)),
            )
        }
    }
}
