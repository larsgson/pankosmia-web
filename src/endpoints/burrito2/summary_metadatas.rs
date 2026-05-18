use crate::gitea::{CuratedOrgs, GiteaCache, GiteaProxyClient};
use crate::store::SharedProjectStore;
use crate::structs::AppSettings;
use crate::structs::MetadataSummary;
use crate::utils::burrito::{summary_metadata_from_file, summary_metadata_from_str};
use crate::utils::paths::os_slash_str;
use crate::utils::response::ok_json_response;
use rocket::http::ContentType;
use rocket::response::status;
use rocket::{get, State};
use std::collections::BTreeMap;
use std::sync::Arc;

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

#[get("/metadata/summaries?<org>")]
pub async fn summary_metadatas(
    _state: &State<AppSettings>,
    store: &State<SharedProjectStore>,
    curated: &State<CuratedOrgs>,
    client: &State<GiteaProxyClient>,
    cache: &State<GiteaCache>,
    org: Option<String>,
) -> status::Custom<(ContentType, String)> {
    let mut repos: BTreeMap<String, MetadataSummary> = BTreeMap::new();

    // Curated orgs: fetch from Gitea API (cached)
    for (server, org_name) in curated.iter_orgs() {
        let server_org = format!("{}/{}", server, org_name);
        if let Some(ref filter) = org {
            if *filter != server_org {
                continue;
            }
        }
        let cache_key = server_org.clone();
        if let Some(cached) = cache.summaries.get(&cache_key) {
            for (k, v) in cached.as_ref() {
                repos.insert(
                    k.clone(),
                    MetadataSummary {
                        name: v.name.clone(),
                        description: v.description.clone(),
                        abbreviation: v.abbreviation.clone(),
                        generated_date: v.generated_date.clone(),
                        flavor_type: v.flavor_type.clone(),
                        flavor: v.flavor.clone(),
                        language_code: v.language_code.clone(),
                        language_name: v.language_name.clone(),
                        script_direction: v.script_direction.clone(),
                        book_codes: v.book_codes.clone(),
                        timestamp: v.timestamp,
                    },
                );
            }
            continue;
        }

        let org_repos = match client.list_org_repos(server, org_name).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("WARN: could not list repos for {}: {}", server_org, e);
                continue;
            }
        };

        let mut org_summaries: BTreeMap<String, MetadataSummary> = BTreeMap::new();
        for repo_val in &org_repos {
            let repo_name = match repo_val.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };
            let repo_key = format!("{}/{}/{}", server, org_name, repo_name);
            let summary = match client
                .fetch_raw(server, org_name, repo_name, "metadata.json", "master")
                .await
            {
                Ok((_ct, bytes)) => match String::from_utf8(bytes) {
                    Ok(json_str) => {
                        summary_metadata_from_str(&json_str).unwrap_or_else(|_| fallback_summary())
                    }
                    Err(_) => fallback_summary(),
                },
                Err(_) => continue,
            };
            org_summaries.insert(repo_key, summary);
        }

        cache
            .summaries
            .insert(cache_key, Arc::new(org_summaries.clone()));
        repos.extend(org_summaries);
    }

    // Local filesystem: walk for non-curated orgs
    let root_path = store.workspace_root().to_string_lossy().into_owned();
    if let Ok(server_paths) = std::fs::read_dir(&root_path) {
        for server_path in server_paths {
            let uw_server_path_ob = match server_path {
                Ok(p) => p.path(),
                Err(_) => continue,
            };
            let server_leaf = match uw_server_path_ob.file_name().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if server_leaf.starts_with('.') || !uw_server_path_ob.is_dir() {
                continue;
            }
            let org_entries = match std::fs::read_dir(&uw_server_path_ob) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for org_path in org_entries {
                let uw_org_path_ob = match org_path {
                    Ok(p) => p.path(),
                    Err(_) => continue,
                };
                let org_leaf = match uw_org_path_ob.file_name().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let server_org = format!("{}/{}", server_leaf, org_leaf);

                // Skip curated orgs (already fetched from Gitea)
                if curated.is_curated(&server_org) {
                    continue;
                }

                if let Some(ref filter) = org {
                    if *filter != server_org {
                        continue;
                    }
                } else {
                    if server_org == "_local_/_quarantine_"
                        || server_org == "_local_/_archive_"
                        || server_org == "_local_/_updates_"
                    {
                        continue;
                    }
                }
                if org_leaf.starts_with('.') || !uw_org_path_ob.is_dir() {
                    continue;
                }
                let repo_entries = match std::fs::read_dir(&uw_org_path_ob) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                for repo_path in repo_entries {
                    let uw_repo_path_ob = match repo_path {
                        Ok(p) => p.path(),
                        Err(_) => continue,
                    };
                    let repo_leaf = match uw_repo_path_ob.file_name().and_then(|s| s.to_str()) {
                        Some(s) => s.to_string(),
                        None => continue,
                    };
                    if repo_leaf.starts_with('.') || !uw_repo_path_ob.is_dir() {
                        continue;
                    }
                    let repo_url_string = format!("{}/{}/{}", server_leaf, org_leaf, repo_leaf);
                    let metadata_path = format!(
                        "{}{}{}{}metadata.json",
                        root_path,
                        os_slash_str(),
                        &repo_url_string,
                        os_slash_str()
                    );
                    let summary = summary_metadata_from_file(metadata_path)
                        .unwrap_or_else(|_| fallback_summary());
                    repos.insert(repo_url_string, summary);
                }
            }
        }
    }

    ok_json_response(serde_json::to_string(&repos).unwrap())
}
