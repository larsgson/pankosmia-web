use crate::gitea::{GiteaProxyClient, CuratedOrgs};
use crate::store::SharedProjectStore;
use crate::utils::response::ok_json_response;
use rocket::http::ContentType;
use rocket::response::status;
use rocket::{get, State};
use std::collections::BTreeSet;

#[get("/list-local-repos")]
pub async fn list_local_repos(
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    client: &State<GiteaProxyClient>,
) -> status::Custom<(ContentType, String)> {
    let mut repos: BTreeSet<String> = BTreeSet::new();

    // Curated orgs: list from Gitea API
    for (server, org) in curated.iter_orgs() {
        if let Ok(org_repos) = client.list_org_repos(server, org).await {
            for repo_val in &org_repos {
                if let Some(name) = repo_val.get("name").and_then(|n| n.as_str()) {
                    repos.insert(format!("{}/{}/{}", server, org, name));
                }
            }
        }
    }

    // Local filesystem walk
    let root = store.workspace_root().to_path_buf();
    let server_paths = match std::fs::read_dir(&root) {
        Ok(p) => p,
        Err(_) => {
            let repos_vec: Vec<String> = repos.into_iter().collect();
            return ok_json_response(serde_json::to_string(&repos_vec).unwrap());
        }
    };
    for server_path in server_paths {
        let uw_server_path = match server_path {
            Ok(e) => e.path(),
            Err(_) => continue,
        };
        let server_leaf = match uw_server_path.file_name() {
            Some(n) => n.to_string_lossy().into_owned(),
            None => continue,
        };
        if server_leaf.starts_with('.') || !uw_server_path.is_dir() {
            continue;
        }
        let org_iter = match std::fs::read_dir(&uw_server_path) {
            Ok(i) => i,
            Err(_) => continue,
        };
        for org_path in org_iter {
            let uw_org_path = match org_path {
                Ok(e) => e.path(),
                Err(_) => continue,
            };
            let org_leaf = match uw_org_path.file_name() {
                Some(n) => n.to_string_lossy().into_owned(),
                None => continue,
            };
            if org_leaf.starts_with('.') || !uw_org_path.is_dir() {
                continue;
            }
            let server_org = format!("{}/{}", server_leaf, org_leaf);
            if server_org == "_local_/_quarantine_"
                || server_org == "_local_/_archive_"
                || server_org == "_local_/_updates_"
            {
                continue;
            }
            // Skip curated orgs (already fetched from Gitea)
            if curated.is_curated(&server_org) {
                continue;
            }
            let repo_iter = match std::fs::read_dir(&uw_org_path) {
                Ok(i) => i,
                Err(_) => continue,
            };
            for repo_path in repo_iter {
                let uw_repo_path = match repo_path {
                    Ok(e) => e.path(),
                    Err(_) => continue,
                };
                let repo_leaf = match uw_repo_path.file_name() {
                    Some(n) => n.to_string_lossy().into_owned(),
                    None => continue,
                };
                if repo_leaf.starts_with('.') || !uw_repo_path.is_dir() {
                    continue;
                }
                repos.insert(format!("{}/{}/{}", server_leaf, org_leaf, repo_leaf));
            }
        }
    }
    let repos_vec: Vec<String> = repos.into_iter().collect();
    ok_json_response(serde_json::to_string(&repos_vec).unwrap())
}
