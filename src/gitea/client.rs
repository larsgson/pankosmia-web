use serde::Deserialize;

const USER_AGENT: &str = "pankosmia-web";

#[derive(Debug, thiserror::Error)]
pub enum GiteaProxyError {
    #[error("network: {0}")]
    Network(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("gitea api {status}: {body}")]
    Api { status: u16, body: String },
    #[error("not found")]
    NotFound,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub size: Option<u64>,
}

pub struct GiteaProxyClient {
    inner: reqwest::Client,
}

impl GiteaProxyClient {
    pub fn new() -> Self {
        let inner = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .expect("reqwest client");
        Self { inner }
    }

    pub async fn fetch_raw(
        &self,
        server: &str,
        org: &str,
        repo: &str,
        path: &str,
        branch: &str,
    ) -> Result<(String, Vec<u8>), GiteaProxyError> {
        let url = format!(
            "https://{}/{}/{}/raw/branch/{}/{}",
            server, org, repo, branch, path
        );
        let resp = self
            .inner
            .get(&url)
            .send()
            .await
            .map_err(|e| GiteaProxyError::Network(e.to_string()))?;
        if resp.status().as_u16() == 404 {
            return Err(GiteaProxyError::NotFound);
        }
        if !resp.status().is_success() {
            return Err(GiteaProxyError::Api {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| GiteaProxyError::Decode(e.to_string()))?
            .to_vec();
        Ok((content_type, bytes))
    }

    pub async fn list_tree(
        &self,
        server: &str,
        org: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Vec<TreeEntry>, GiteaProxyError> {
        let url = format!(
            "https://{}/api/v1/repos/{}/{}/git/trees/{}?recursive=true",
            server, org, repo, branch
        );
        let resp = self
            .inner
            .get(&url)
            .send()
            .await
            .map_err(|e| GiteaProxyError::Network(e.to_string()))?;
        if resp.status().as_u16() == 404 {
            return Err(GiteaProxyError::NotFound);
        }
        if !resp.status().is_success() {
            return Err(GiteaProxyError::Api {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| GiteaProxyError::Decode(e.to_string()))?;
        let entries: Vec<TreeEntry> =
            serde_json::from_value(body.get("tree").cloned().unwrap_or(serde_json::json!([])))
                .map_err(|e| GiteaProxyError::Decode(e.to_string()))?;
        Ok(entries)
    }

    pub async fn fetch_archive(
        &self,
        server: &str,
        org: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Vec<u8>, GiteaProxyError> {
        let url = format!(
            "https://{}/api/v1/repos/{}/{}/archive/{}.zip",
            server, org, repo, branch
        );
        let resp = self
            .inner
            .get(&url)
            .send()
            .await
            .map_err(|e| GiteaProxyError::Network(e.to_string()))?;
        if resp.status().as_u16() == 404 {
            return Err(GiteaProxyError::NotFound);
        }
        if !resp.status().is_success() {
            return Err(GiteaProxyError::Api {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| GiteaProxyError::Decode(e.to_string()))?
            .to_vec();
        Ok(bytes)
    }

    pub async fn list_org_repos(
        &self,
        server: &str,
        org: &str,
    ) -> Result<Vec<serde_json::Value>, GiteaProxyError> {
        let mut all_repos = Vec::new();
        let mut page = 1u32;
        loop {
            let url = format!(
                "https://{}/api/v1/orgs/{}/repos?limit=50&page={}",
                server, org, page
            );
            let resp = self
                .inner
                .get(&url)
                .send()
                .await
                .map_err(|e| GiteaProxyError::Network(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(GiteaProxyError::Api {
                    status: resp.status().as_u16(),
                    body: resp.text().await.unwrap_or_default(),
                });
            }
            let repos: Vec<serde_json::Value> = resp
                .json()
                .await
                .map_err(|e| GiteaProxyError::Decode(e.to_string()))?;
            if repos.is_empty() {
                break;
            }
            all_repos.extend(repos);
            page += 1;
            if page > 20 {
                break;
            }
        }
        Ok(all_repos)
    }
}
