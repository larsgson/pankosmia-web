use crate::gitea::{resolve_read_source, GiteaProxyClient, CuratedOrgs, ReadSource};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::structs::BytesOrError;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::zip::make_zip_file;
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};
use std::path::{Components, PathBuf};

#[get("/zipped/<repo_path..>")]
pub async fn get_zipped_repo(
    _state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    client: &State<GiteaProxyClient>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, BytesOrError)> {
    match resolve_read_source(curated, &repo_path) {
        ReadSource::Gitea(parsed) => {
            match client.fetch_archive(&parsed.server, &parsed.org, &parsed.repo, "master").await {
                Ok(bytes) => status::Custom(Status::Ok, (ContentType::ZIP, BytesOrError::Bytes(bytes))),
                Err(e) => status::Custom(
                    Status::BadGateway,
                    (
                        ContentType::JSON,
                        BytesOrError::Error(make_bad_json_data_response(format!("gitea proxy: {}", e))),
                    ),
                ),
            }
        }
        ReadSource::LocalFilesystem => {
            let path_components: Components<'_> = repo_path.components();
            if check_path_components(&mut path_components.clone()) {
                let path_to_repo = format!(
                    "{}{}{}",
                    store.workspace_root().to_string_lossy().into_owned(),
                    os_slash_str(),
                    &repo_path.display().to_string()
                );
                if !std::path::Path::new(&path_to_repo).is_dir() {
                    return status::Custom(
                        Status::BadRequest,
                        (
                            ContentType::JSON,
                            BytesOrError::Error(make_bad_json_data_response(
                                "could not locate repo".to_string(),
                            )),
                        ),
                    );
                }
                let temp_zip_path = make_zip_file(&path_to_repo);
                match std::fs::read(&temp_zip_path) {
                    Ok(b) => status::Custom(Status::Ok, (ContentType::ZIP, BytesOrError::Bytes(b))),
                    Err(e) => status::Custom(
                        Status::InternalServerError,
                        (
                            ContentType::JSON,
                            BytesOrError::Error(make_bad_json_data_response(format!(
                                "Could not read zip: {}",
                                e
                            ))),
                        ),
                    ),
                }
            } else {
                status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        BytesOrError::Error(make_bad_json_data_response("bad repo path".to_string())),
                    ),
                )
            }
        }
    }
}
