use crate::store::SharedProjectStore;
use crate::utils::response::ok_json_response;
use rocket::http::ContentType;
use rocket::response::status;
use rocket::{get, State};

/// *`GET /list-local-repos`*
///
/// Typically mounted as **`/git/list-local-repos`**
///
/// Returns a JSON array of local repo paths in the legacy
/// `<source>/<org>/<name>` form.
///
/// Walks the workspace root from the `ProjectStore` (M3) instead of
/// reaching directly into `state.repo_dir`. The reserved
/// `.pankosmia/` directory is skipped naturally because it
/// dot-prefixes — same predicate the original code used. Once the
/// hosted Phase 2 backend lands, this enumeration moves into the
/// trait so endpoints don't read the FS at all.
#[get("/list-local-repos")]
pub fn list_local_repos(
    store: &State<SharedProjectStore>,
) -> status::Custom<(ContentType, String)> {
    let root = store.workspace_root().to_path_buf();
    let mut repos: Vec<String> = Vec::new();
    let server_paths = match std::fs::read_dir(&root) {
        Ok(p) => p,
        Err(_) => return ok_json_response("[]".to_string()),
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
        if server_leaf.starts_with('.') {
            continue;
        }
        if !uw_server_path.is_dir() {
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
            if org_leaf.starts_with('.') {
                continue;
            }
            if !uw_org_path.is_dir() {
                continue;
            }
            let server_org = format!("{}/{}", server_leaf, org_leaf);
            if server_org == "_local_/_quarantine_"
                || server_org == "_local_/_archive_"
                || server_org == "_local_/_updates_"
            {
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
                if repo_leaf.starts_with('.') {
                    continue;
                }
                if !uw_repo_path.is_dir() {
                    continue;
                }
                repos.push(format!("{}/{}/{}", server_leaf, org_leaf, repo_leaf));
            }
        }
    }
    ok_json_response(serde_json::to_string(&repos).unwrap())
}
