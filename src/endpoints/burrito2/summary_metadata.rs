use crate::gitea::{resolve_read_source, GiteaProxyClient, CuratedOrgs, ReadSource};
use crate::store::SharedProjectStore;
use crate::structs::{AppSettings, MetadataSummary};
use crate::utils::burrito::{summary_metadata_from_file, summary_metadata_from_str};
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::paths::{check_path_components, os_slash_str};
use crate::utils::response::{
    not_ok_bad_repo_json_response, not_ok_json_response, ok_json_response,
};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, State};
use std::path::{Components, PathBuf};

fn fallback_summary() -> MetadataSummary {
    MetadataSummary {
        name: "? Bad Metadata JSON ?".to_string(),
        description: "?".to_string(),
        abbreviation: "?".to_string(),
        generated_date: "?".to_string(),
        flavor_type: "?".to_string(),
        flavor: "?".to_string(),
        language_code: "?".to_string(),
        language_name: "?".to_string(),
        script_direction: "?".to_string(),
        book_codes: vec![],
        timestamp: 0,
    }
}

#[get("/metadata/summary/<repo_path..>")]
pub async fn summary_metadata(
    _state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    client: &State<GiteaProxyClient>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    match resolve_read_source(curated, &repo_path) {
        ReadSource::Gitea(parsed) => {
            let summary = match client
                .fetch_raw(&parsed.server, &parsed.org, &parsed.repo, "metadata.json", "master")
                .await
            {
                Ok((_ct, bytes)) => match String::from_utf8(bytes) {
                    Ok(json_str) => summary_metadata_from_str(&json_str).unwrap_or_else(|_| fallback_summary()),
                    Err(_) => fallback_summary(),
                },
                Err(_) => fallback_summary(),
            };
            match serde_json::to_string(&summary) {
                Ok(v) => ok_json_response(v),
                Err(e) => not_ok_json_response(
                    Status::InternalServerError,
                    make_bad_json_data_response(format!("could not serialize metadata: {}", e)),
                ),
            }
        }
        ReadSource::LocalFilesystem => {
            let path_components: Components<'_> = repo_path.components();
            if check_path_components(&mut path_components.clone()) {
                let path_to_serve = format!(
                    "{}{}{}{}metadata.json",
                    store.workspace_root().to_string_lossy().into_owned(),
                    os_slash_str(),
                    &repo_path.display().to_string(),
                    os_slash_str()
                );
                let summary = summary_metadata_from_file(path_to_serve)
                    .unwrap_or_else(|_| fallback_summary());
                match serde_json::to_string(&summary) {
                    Ok(v) => ok_json_response(v),
                    Err(e) => not_ok_json_response(
                        Status::InternalServerError,
                        make_bad_json_data_response(format!("could not serialize metadata: {}", e)),
                    ),
                }
            } else {
                not_ok_bad_repo_json_response()
            }
        }
    }
}
