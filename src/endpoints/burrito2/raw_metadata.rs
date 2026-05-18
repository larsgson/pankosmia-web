use crate::gitea::{resolve_read_source, GiteaProxyClient, CuratedOrgs, ReadSource};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_json_response,
};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};
use std::path::{Components, PathBuf};

#[get("/metadata/raw/<repo_path..>")]
pub async fn raw_metadata(
    _state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    client: &State<GiteaProxyClient>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    match resolve_read_source(curated, &repo_path) {
        ReadSource::Gitea(parsed) => {
            match client.fetch_raw(&parsed.server, &parsed.org, &parsed.repo, "metadata.json", "master").await {
                Ok((_ct, bytes)) => {
                    match String::from_utf8(bytes) {
                        Ok(json_str) => ok_json_response(json_str),
                        Err(e) => not_ok_json_response(
                            Status::BadGateway,
                            make_bad_json_data_response(format!("not valid UTF-8: {}", e)),
                        ),
                    }
                }
                Err(e) => not_ok_json_response(
                    Status::BadGateway,
                    make_bad_json_data_response(format!("gitea proxy: {}", e)),
                ),
            }
        }
        ReadSource::LocalFilesystem => {
            let path_components: Components<'_> = repo_path.components();
            if check_path_components(&mut path_components.clone()) {
                let path_to_serve = store.workspace_root().to_string_lossy().into_owned()
                    + os_slash_str()
                    + &repo_path.display().to_string()
                    + "/metadata.json";
                match std::fs::read_to_string(path_to_serve) {
                    Ok(v) => ok_json_response(v),
                    Err(e) => not_ok_json_response(
                        Status::BadRequest,
                        make_bad_json_data_response(format!("could not read metadata: {}", e)),
                    ),
                }
            } else {
                not_ok_bad_repo_json_response()
            }
        }
    }
}
