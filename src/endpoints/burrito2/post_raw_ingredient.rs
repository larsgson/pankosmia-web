use crate::auth::{GithubAppAuth, GithubClient, LanguageHeader, TokenStore};
use crate::endpoints::burrito2::github_save::{
    handle_github_op, is_github_backend, validate_ipath_segments,
};
use crate::server::{LanguageLocks, RateLimiter};
use crate::store::github::{GithubEditFlow, SaveOp};
use crate::structs::{AppSettings, BurritoMetadata};
use crate::utils::burrito::{
    destination_parent, ingredients_metadata_from_files, ingredients_scopes_from_files,
};
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, check_path_string_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_ok_json_response,
};
use rocket::http::{ContentType, CookieJar, Status};
use rocket::response::status;
use rocket::serde::json::Json;
use rocket::{post, State};
use serde_json::Value;
use std::path::{Components, PathBuf};

/// *`POST /ingredient/raw/<repo_path>?ipath=my_burrito_path&update_ingredients&no_bak`*
///
/// Typically mounted as **`/burrito/ingredient/raw/<repo_path>?ipath=my_burrito_path&update_ingredients&no_bak`**
///
/// Writes a document, where the document is provided as JSON with a 'payload' key. The ipath parameter is required. There are two optional parameters:
/// - update_ingredients to rewrite the metadata (default is false)
/// - no_bak to write bak files (default is true)
///
/// Backend dispatch: when `STORAGE_BACKEND=github`, the request is
/// authenticated via the session cookie, the language is read from
/// the `X-Language-Code` header, and the edit flow forks/branches/
/// pushes/PRs against the upstream language repo. Otherwise the
/// legacy FS write path runs.

#[post(
    "/ingredient/raw/<repo_path..>?<ipath>&<update_ingredients>&<no_bak>",
    format = "json",
    data = "<json_form>"
)]
#[allow(irrefutable_let_patterns)]
pub async fn post_raw_ingredient(
    state: &State<AppSettings>,
    cookies: &CookieJar<'_>,
    edit_flow: &State<GithubEditFlow>,
    app_auth: Option<&State<GithubAppAuth>>,
    tokens: &State<TokenStore>,
    github_client: &State<GithubClient>,
    locks: &State<LanguageLocks>,
    rate_limiter: &State<RateLimiter>,
    language_header: Option<LanguageHeader>,
    repo_path: PathBuf,
    ipath: String,
    update_ingredients: Option<String>,
    no_bak: Option<String>,
    json_form: Json<Value>,
) -> status::Custom<(ContentType, String)> {
    if is_github_backend() {
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
        return handle_github_op(
            cookies,
            edit_flow,
            app_auth,
            tokens,
            github_client,
            locks,
            rate_limiter,
            language_header,
            SaveOp::Put { ipath: &ipath, bytes: &bytes },
            &commit_message,
        )
        .await;
    }
    let path_components: Components<'_> = repo_path.components();
    let repo_dir = state.repo_dir.lock().expect("lock for repo dir");
    let full_repo_path =
        format!("{}{}{}", &repo_dir, os_slash_str(), &repo_path.display().to_string());
    if check_path_components(&mut path_components.clone())
        && check_path_string_components(ipath.clone())
        && std::fs::metadata(&full_repo_path).is_ok()
    {
        let destination = format!("{}{}ingredients{}{}", &full_repo_path, os_slash_str(), os_slash_str(), &ipath);
        let destination_parent = destination_parent(destination.clone());
        // Make subdirs if necessary
        if !std::path::Path::new(&destination_parent).exists() {
            match std::fs::create_dir_all(destination_parent) {
                Ok(_) => (),
                Err(e) => {
                    return not_ok_json_response(
                        Status::InternalServerError,
                        make_bad_json_data_response(format!(
                            "Could not create local content directories: {}",
                            e
                        )),
                    )
                }
            }
        }
        // Maybe make backup file
        if !no_bak.is_some() {
            let destination_backup_path = format!("{}.bak", &destination);
            if std::path::Path::new(&destination).exists() {
                match std::fs::rename(&destination, &destination_backup_path) {
                    Ok(_) => (),
                    Err(e) => {
                        return not_ok_json_response(
                            Status::InternalServerError,
                            make_bad_json_data_response(format!("Could not write backup file: {}", e)),
                        )
                    }
                }
            }
        }
        match std::fs::write(destination, json_form["payload"].as_str().unwrap()) {
            Ok(_) => {},
            Err(e) => return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("Could not write to {}: {}", ipath, e)),
            ),
        };
        if update_ingredients.is_some() {
            // Get metadata as struct
            let app_resources_dir = format!("{}", &state.app_resources_dir);
            let path_to_repo_metadata = format!(
                "{}{}metadata.json",
                &full_repo_path,
                os_slash_str(),
            );
            let metadata_string = match std::fs::read_to_string(&path_to_repo_metadata) {
                Ok(v) => v,
                Err(e) => {
                    return not_ok_json_response(
                        Status::InternalServerError,
                        make_bad_json_data_response(format!(
                            "Could not load metadata as string: {}",
                            e
                        )),
                    )
                }
            };
            // Make struct from metadata
            let mut metadata_struct: BurritoMetadata = match serde_json::from_str(&metadata_string) {
                Ok(v) => v,
                Err(e) => {
                    return not_ok_json_response(
                        Status::InternalServerError,
                        make_bad_json_data_response(format!("Could not parse metadata: {}", e)),
                    );
                }
            };
            // Add ingredient record and currentScope value for USFM
            if let mut ingredients = metadata_struct.ingredients.lock().unwrap() {
                let new_ingredients = ingredients_metadata_from_files(app_resources_dir.clone(), full_repo_path.clone());
                *ingredients = new_ingredients;
            }
            if let type_info = metadata_struct.r#type {
                let mut type_ob = type_info.as_object().unwrap().clone();
                let flavor_type_ob = type_ob["flavorType"].as_object_mut().unwrap();
                let new_current_scope = ingredients_scopes_from_files(app_resources_dir, full_repo_path.clone());
                flavor_type_ob["currentScope"] = serde_json::from_str(serde_json::to_string(&new_current_scope).unwrap().as_str()).unwrap();
                metadata_struct.r#type = serde_json::from_str(serde_json::to_string(&type_ob).unwrap().as_str()).unwrap();
            }

            // Write metadata
            let metadata_output_string = match serde_json::to_string(&metadata_struct) {
                Ok(s) => s,
                Err(e) => {
                    return not_ok_json_response(
                        Status::InternalServerError,
                        make_bad_json_data_response(format!("Could not make metadata as JSON: {}", e)),
                    )
                }
            };
            match std::fs::write(path_to_repo_metadata, &metadata_output_string) {
                Ok(_) => (),
                Err(e) => {
                    return not_ok_json_response(
                        Status::InternalServerError,
                        make_bad_json_data_response(format!("Could not write metadata to repo: {}", e)),
                    )
                }
            }
        }
        ok_ok_json_response()
    } else {
        not_ok_bad_repo_json_response()
    }
}

