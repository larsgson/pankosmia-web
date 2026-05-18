use crate::gitea::{resolve_read_source, CuratedOrgs, GiteaProxyClient, ReadSource};
use crate::store::SharedProjectStore;
use crate::structs::{AppSettings, BytesOrError};
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::mime::mime_types;
use crate::utils::paths::{check_path_components, check_path_string_components, os_slash_str};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};
use std::path::{Components, PathBuf};

#[get("/ingredient/bytes/<repo_path..>?<ipath>")]
pub async fn raw_bytes_ingredient(
    _state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    client: &State<GiteaProxyClient>,
    repo_path: PathBuf,
    ipath: String,
) -> status::Custom<(ContentType, BytesOrError)> {
    if !check_path_string_components(ipath.clone()) {
        return status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                BytesOrError::Error(make_bad_json_data_response("bad repo path".to_string())),
            ),
        );
    }

    match resolve_read_source(curated, &repo_path) {
        ReadSource::Gitea(parsed) => {
            match client
                .fetch_raw(&parsed.server, &parsed.org, &parsed.repo, &ipath, "master")
                .await
            {
                Ok((_content_type, bytes)) => {
                    let mut split_ipath = ipath.split('.');
                    let mut suffix = "unknown";
                    if let Some(_) = split_ipath.next() {
                        if let Some(second) = split_ipath.next() {
                            suffix = second;
                        }
                    }
                    status::Custom(
                        Status::Ok,
                        (
                            match mime_types().get(suffix) {
                                Some(t) => t.clone(),
                                None => ContentType::new("application", "octet-stream"),
                            },
                            BytesOrError::Bytes(bytes),
                        ),
                    )
                }
                Err(e) => status::Custom(
                    Status::BadGateway,
                    (
                        ContentType::JSON,
                        BytesOrError::Error(make_bad_json_data_response(format!(
                            "gitea proxy: {}",
                            e
                        ))),
                    ),
                ),
            }
        }
        ReadSource::LocalFilesystem => {
            let path_components: Components<'_> = repo_path.components();
            if !check_path_components(&mut path_components.clone()) {
                return status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        BytesOrError::Error(make_bad_json_data_response(
                            "bad repo path".to_string(),
                        )),
                    ),
                );
            }
            let path_to_serve = store.workspace_root().to_string_lossy().into_owned()
                + os_slash_str()
                + &repo_path.display().to_string()
                + "/ingredients/"
                + ipath.as_str();
            match std::fs::read(path_to_serve) {
                Ok(v) => {
                    let mut split_ipath = ipath.split('.');
                    let mut suffix = "unknown";
                    if let Some(_) = split_ipath.next() {
                        if let Some(second) = split_ipath.next() {
                            suffix = second;
                        }
                    }
                    status::Custom(
                        Status::Ok,
                        (
                            match mime_types().get(suffix) {
                                Some(t) => t.clone(),
                                None => ContentType::new("application", "octet-stream"),
                            },
                            BytesOrError::Bytes(v),
                        ),
                    )
                }
                Err(e) => status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        BytesOrError::Error(make_bad_json_data_response(
                            format!("could not read ingredient content: {}", e).to_string(),
                        )),
                    ),
                ),
            }
        }
    }
}
