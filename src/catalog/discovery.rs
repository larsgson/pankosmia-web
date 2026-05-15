//! Org-based catalog auto-discovery.
//!
//! When `PANKOSMIA_CATALOG_ORG` is set the server discovers languages
//! by searching the GitHub API for repos with topic
//! `pankosmia-language` in that org, then fetching each repo's
//! `language.yaml` for metadata. No central `languages.yaml` needed.

use crate::auth::github_app::GithubAppAuth;
use crate::catalog::registry::{CatalogRegistry, RegisteredLanguage, RegistryDiff};
use crate::identity::LanguageCode;
use serde::Deserialize;

const GITHUB_API: &str = "https://api.github.com";
const USER_AGENT: &str = "pankosmia-web";

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("auth: {0}")]
    Auth(String),
    #[error("network: {0}")]
    Network(String),
    #[error("github api {status}: {body}")]
    Api { status: u16, body: String },
    #[error("decode: {0}")]
    Decode(String),
    #[error("repo {repo}: {reason}")]
    BadRepo { repo: String, reason: String },
}

/// Shape of `language.yaml` in each language repo.
#[derive(Debug, Deserialize)]
struct LanguageYaml {
    bcp47: String,
    english_name: String,
    #[allow(dead_code)]
    native_name: Option<String>,
    direction: Option<String>,
    script: Option<String>,
}

/// Minimal fields from the GitHub Search API response.
#[derive(Debug, Deserialize)]
struct SearchResponse {
    items: Vec<SearchItem>,
    total_count: u64,
}

#[derive(Debug, Deserialize)]
struct SearchItem {
    full_name: String, // "pankosmia-langs/yua"
}

/// Discover all language repos in `org` and reload the catalog.
///
/// 1. Mint an installation token via `app_auth`.
/// 2. Paginate `GET /search/repositories?q=org:{org}+topic:pankosmia-language`.
/// 3. For each repo, fetch `language.yaml` via the Contents API.
/// 4. Atomically swap the registry contents.
///
/// Repos whose `language.yaml` is missing or unparseable are logged
/// and skipped — a single bad repo does not block the rest.
pub async fn discover_languages(
    app_auth: &GithubAppAuth,
    installation_id: u64,
    org: &str,
    catalog: &CatalogRegistry,
) -> Result<RegistryDiff, DiscoveryError> {
    let token = app_auth
        .installation_token(installation_id)
        .await
        .map_err(|e| DiscoveryError::Auth(e.to_string()))?;

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| DiscoveryError::Network(e.to_string()))?;

    let repos = search_org_repos(&client, &token, org).await?;
    println!(
        "[discovery] found {} repos in org {} with topic pankosmia-language",
        repos.len(),
        org
    );

    let mut entries = Vec::new();
    for repo_full_name in &repos {
        match fetch_language_yaml(&client, &token, repo_full_name).await {
            Ok(lang) => {
                let code = match LanguageCode::parse(&lang.bcp47) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!(
                            "[discovery] {}: bad bcp47 {:?}: {}",
                            repo_full_name, lang.bcp47, e
                        );
                        continue;
                    }
                };
                entries.push(RegisteredLanguage {
                    code,
                    display_name: lang.english_name,
                    repo: repo_full_name.clone(),
                    script: lang.script,
                    direction: lang.direction,
                    installation_id: Some(installation_id),
                });
            }
            Err(e) => {
                eprintln!("[discovery] {}: skipping: {}", repo_full_name, e);
            }
        }
    }

    println!("[discovery] loaded {} languages", entries.len());
    Ok(catalog.reload_from_entries(entries))
}

/// Paginate the GitHub Search API to find all repos in `org` with
/// topic `pankosmia-language`.
async fn search_org_repos(
    client: &reqwest::Client,
    token: &str,
    org: &str,
) -> Result<Vec<String>, DiscoveryError> {
    let mut repos = Vec::new();
    let mut page = 1u32;
    loop {
        let url = format!(
            "{}/search/repositories?q=org:{}+topic:pankosmia-language&per_page=100&page={}",
            GITHUB_API, org, page
        );
        let resp = client
            .get(&url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| DiscoveryError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(DiscoveryError::Api { status, body });
        }
        let page_data: SearchResponse = resp
            .json()
            .await
            .map_err(|e| DiscoveryError::Decode(e.to_string()))?;
        let page_len = page_data.items.len();
        for item in page_data.items {
            repos.push(item.full_name);
        }
        if repos.len() as u64 >= page_data.total_count || page_len < 100 {
            break;
        }
        page += 1;
    }
    Ok(repos)
}

/// Fetch and parse `language.yaml` from the default branch of a repo.
async fn fetch_language_yaml(
    client: &reqwest::Client,
    token: &str,
    repo: &str,
) -> Result<LanguageYaml, DiscoveryError> {
    let url = format!("{}/repos/{}/contents/language.yaml", GITHUB_API, repo);
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github.raw+json")
        .send()
        .await
        .map_err(|e| DiscoveryError::Network(e.to_string()))?;
    if resp.status().as_u16() == 404 {
        return Err(DiscoveryError::BadRepo {
            repo: repo.into(),
            reason: "language.yaml not found".into(),
        });
    }
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(DiscoveryError::Api { status, body });
    }
    let raw = resp
        .text()
        .await
        .map_err(|e| DiscoveryError::Decode(e.to_string()))?;
    let lang: LanguageYaml = serde_yaml::from_str(&raw).map_err(|e| DiscoveryError::BadRepo {
        repo: repo.into(),
        reason: format!("language.yaml parse: {}", e),
    })?;
    Ok(lang)
}
