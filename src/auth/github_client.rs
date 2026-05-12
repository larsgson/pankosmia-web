//! Thin wrapper around `reqwest` for the small set of GitHub API
//! calls the server makes.
//!
//! Calls are made on behalf of a user (with their OAuth access
//! token). The wrapper does not cache responses; cache layers live
//! one level up (e.g. `MembershipCache` for collaborator lookups).
//!
//! Errors are mapped to a single `GithubError` enum so endpoints
//! can render meaningful HTTP responses without each one knowing
//! the GitHub API's error shapes.

use serde::{Deserialize, Serialize};

const GITHUB_API: &str = "https://api.github.com";
const GITHUB_OAUTH_TOKEN: &str = "https://github.com/login/oauth/access_token";
const USER_AGENT: &str = "pankosmia-web";

#[derive(Debug, thiserror::Error)]
pub enum GithubError {
    #[error("network: {0}")]
    Network(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("github api {status}: {body}")]
    Api { status: u16, body: String },
    #[error("oauth code rejected by github")]
    OAuthCodeRejected,
    #[error("token revoked or expired")]
    TokenRevoked,
    #[error("rate limited; retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },
    #[error("not found")]
    NotFound,
}

#[derive(Clone)]
pub struct GithubClient {
    inner: reqwest::Client,
    pub client_id: String,
    pub client_secret: String,
}

impl GithubClient {
    pub fn new(client_id: String, client_secret: String) -> Self {
        let inner = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .expect("reqwest client");
        Self {
            inner,
            client_id,
            client_secret,
        }
    }

    /// Exchange an OAuth code for an access token.
    pub async fn exchange_oauth_code(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<OAuthTokenResponse, GithubError> {
        let resp = self
            .inner
            .post(GITHUB_OAUTH_TOKEN)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(GithubError::Api {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let body: OAuthTokenResponseRaw = resp
            .json()
            .await
            .map_err(|e| GithubError::Decode(e.to_string()))?;
        if let Some(err) = body.error {
            // GitHub returns 200 even for OAuth errors; the body's
            // `error` field is the real signal.
            return Err(match err.as_str() {
                "bad_verification_code" | "incorrect_client_credentials" => {
                    GithubError::OAuthCodeRejected
                }
                _ => GithubError::Api {
                    status: 200,
                    body: format!("{}: {}", err, body.error_description.unwrap_or_default()),
                },
            });
        }
        Ok(OAuthTokenResponse {
            access_token: body
                .access_token
                .ok_or_else(|| GithubError::Decode("no access_token in response".into()))?,
            refresh_token: body.refresh_token,
            scope: body.scope.unwrap_or_default(),
            token_type: body.token_type.unwrap_or_default(),
        })
    }

    /// Fetch the authenticated user's profile via `GET /user`.
    pub async fn get_user(&self, token: &str) -> Result<GithubUser, GithubError> {
        let resp = self
            .inner
            .get(format!("{}/user", GITHUB_API))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        map_status(&resp)?;
        resp.json::<GithubUser>()
            .await
            .map_err(|e| GithubError::Decode(e.to_string()))
    }

    /// `GET /repos/{owner}/{repo}` — used by the catalog validator
    /// and by per-repo permission lookups.
    pub async fn get_repo(&self, token: &str, repo: &str) -> Result<GithubRepo, GithubError> {
        let resp = self
            .inner
            .get(format!("{}/repos/{}", GITHUB_API, repo))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        map_status(&resp)?;
        resp.json::<GithubRepo>()
            .await
            .map_err(|e| GithubError::Decode(e.to_string()))
    }

    /// `GET /repos/{owner}/{repo}/collaborators/{username}/permission` —
    /// returns the calling user's permission level. None if not a
    /// collaborator (404).
    pub async fn get_repo_permission(
        &self,
        token: &str,
        repo: &str,
        username: &str,
    ) -> Result<Option<GithubPermission>, GithubError> {
        let resp = self
            .inner
            .get(format!(
                "{}/repos/{}/collaborators/{}/permission",
                GITHUB_API, repo, username
            ))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        map_status(&resp)?;
        resp.json::<GithubPermission>()
            .await
            .map(Some)
            .map_err(|e| GithubError::Decode(e.to_string()))
    }

    /// `POST /repos/{upstream}/pulls` — open a pull request.
    pub async fn open_pull_request(
        &self,
        token: &str,
        upstream: &str,
        title: &str,
        head: &str,
        base: &str,
        body: &str,
    ) -> Result<GithubPullRequest, GithubError> {
        #[derive(Serialize)]
        struct Body<'a> {
            title: &'a str,
            head: &'a str,
            base: &'a str,
            body: &'a str,
        }
        let resp = self
            .inner
            .post(format!("{}/repos/{}/pulls", GITHUB_API, upstream))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .json(&Body { title, head, base, body })
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        map_status(&resp)?;
        resp.json::<GithubPullRequest>()
            .await
            .map_err(|e| GithubError::Decode(e.to_string()))
    }

    /// `PUT /repos/{upstream}/pulls/{n}/merge` — merge a PR.
    pub async fn merge_pull_request(
        &self,
        token: &str,
        upstream: &str,
        pr_number: u64,
    ) -> Result<(), GithubError> {
        let resp = self
            .inner
            .put(format!("{}/repos/{}/pulls/{}/merge", GITHUB_API, upstream, pr_number))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("Content-Length", "0")
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        map_status(&resp)?;
        Ok(())
    }

    /// `GET /repos/{upstream}/pulls?head={owner}:{branch}&base={base}&state={state}`
    /// — list PRs filtered by head, base, and state. Used by the
    /// edit flow to find an existing open PR for a user's working
    /// branch before opening a new one.
    pub async fn list_pulls(
        &self,
        token: &str,
        upstream: &str,
        head: Option<&str>,
        base: Option<&str>,
        state: &str,
    ) -> Result<Vec<GithubPullRequest>, GithubError> {
        let mut url = format!("{}/repos/{}/pulls?state={}", GITHUB_API, upstream, state);
        if let Some(h) = head {
            url.push_str(&format!("&head={}", urlencoding::encode(h)));
        }
        if let Some(b) = base {
            url.push_str(&format!("&base={}", urlencoding::encode(b)));
        }
        let resp = self
            .inner
            .get(url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        map_status(&resp)?;
        resp.json::<Vec<GithubPullRequest>>()
            .await
            .map_err(|e| GithubError::Decode(e.to_string()))
    }

}

// --- Git Data / Contents API write helpers (App-flow) --------------
//
// These work with any bearer token that has `contents: write` on the
// target repo — typically a GitHub App installation token, but a
// user-to-server token with the same permission works too. Same
// `map_status` mapping as the rest of the client.

impl GithubClient {
    /// `GET /repos/{repo}/git/ref/heads/{branch}` — current SHA of a
    /// branch. Returns `Ok(None)` on 404.
    pub async fn get_branch_sha(
        &self,
        token: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<String>, GithubError> {
        let url = format!("{}/repos/{}/git/ref/heads/{}", GITHUB_API, repo, branch);
        let resp = self
            .inner
            .get(url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        map_status(&resp)?;
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| GithubError::Decode(e.to_string()))?;
        let sha = v
            .pointer("/object/sha")
            .and_then(|s| s.as_str())
            .ok_or_else(|| GithubError::Decode("no /object/sha in ref response".into()))?
            .to_string();
        Ok(Some(sha))
    }

    /// `POST /repos/{repo}/git/refs` — create a branch pointing at
    /// `sha`. Caller must ensure the branch does not already exist
    /// (or check the 422 reply from GitHub).
    pub async fn create_branch(
        &self,
        token: &str,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<(), GithubError> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(rename = "ref")]
            ref_: String,
            sha: &'a str,
        }
        let resp = self
            .inner
            .post(format!("{}/repos/{}/git/refs", GITHUB_API, repo))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .json(&Body {
                ref_: format!("refs/heads/{}", branch),
                sha,
            })
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        map_status(&resp)?;
        Ok(())
    }

    /// `PATCH /repos/{repo}/git/refs/heads/{branch}` — point an
    /// existing branch at `sha`. `force=true` allows non-fast-forward
    /// updates (used to reset the working branch back to upstream
    /// HEAD between saves).
    pub async fn update_branch(
        &self,
        token: &str,
        repo: &str,
        branch: &str,
        sha: &str,
        force: bool,
    ) -> Result<(), GithubError> {
        #[derive(Serialize)]
        struct Body<'a> {
            sha: &'a str,
            force: bool,
        }
        let resp = self
            .inner
            .patch(format!(
                "{}/repos/{}/git/refs/heads/{}",
                GITHUB_API, repo, branch
            ))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .json(&Body { sha, force })
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        map_status(&resp)?;
        Ok(())
    }

    /// `GET /repos/{repo}/contents/{path}?ref={ref}` — fetch file
    /// metadata for a path. Returns `Ok(None)` on 404 (file absent
    /// at that ref). The `sha` field is the blob SHA, which the
    /// `PUT /contents/{path}` call needs to update (not create) the
    /// file in place.
    pub async fn get_file_blob_sha(
        &self,
        token: &str,
        repo: &str,
        path: &str,
        ref_: &str,
    ) -> Result<Option<String>, GithubError> {
        let url = format!(
            "{}/repos/{}/contents/{}?ref={}",
            GITHUB_API,
            repo,
            path,
            urlencoding::encode(ref_)
        );
        let resp = self
            .inner
            .get(url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        map_status(&resp)?;
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| GithubError::Decode(e.to_string()))?;
        let sha = v
            .get("sha")
            .and_then(|s| s.as_str())
            .ok_or_else(|| GithubError::Decode("no sha in contents response".into()))?
            .to_string();
        Ok(Some(sha))
    }

    /// `PUT /repos/{repo}/contents/{path}` — create or update a file
    /// in one call. If `existing_blob_sha` is `Some`, this is an
    /// update (must match the current blob SHA on the target branch).
    /// If `None`, this is a create (target file must not exist).
    /// Returns the resulting commit SHA.
    pub async fn put_file_contents(
        &self,
        token: &str,
        repo: &str,
        path: &str,
        branch: &str,
        content: &[u8],
        message: &str,
        existing_blob_sha: Option<&str>,
    ) -> Result<String, GithubError> {
        use base64::Engine as _;
        #[derive(Serialize)]
        struct Body<'a> {
            message: &'a str,
            content: String,
            branch: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            sha: Option<&'a str>,
        }
        let body = Body {
            message,
            content: base64::engine::general_purpose::STANDARD.encode(content),
            branch,
            sha: existing_blob_sha,
        };
        let resp = self
            .inner
            .put(format!("{}/repos/{}/contents/{}", GITHUB_API, repo, path))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .json(&body)
            .send()
            .await
            .map_err(|e| GithubError::Network(e.to_string()))?;
        map_status(&resp)?;
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| GithubError::Decode(e.to_string()))?;
        let sha = v
            .pointer("/commit/sha")
            .and_then(|s| s.as_str())
            .ok_or_else(|| GithubError::Decode("no /commit/sha in PUT contents response".into()))?
            .to_string();
        Ok(sha)
    }
}

fn map_status(resp: &reqwest::Response) -> Result<(), GithubError> {
    let s = resp.status();
    if s.is_success() {
        return Ok(());
    }
    if s.as_u16() == 401 {
        return Err(GithubError::TokenRevoked);
    }
    if s.as_u16() == 404 {
        return Err(GithubError::NotFound);
    }
    if s.as_u16() == 403 || s.as_u16() == 429 {
        // Rate limit info is in headers.
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);
        return Err(GithubError::RateLimited {
            retry_after_seconds: retry_after,
        });
    }
    Err(GithubError::Api {
        status: s.as_u16(),
        body: String::new(),
    })
}

// --- response shapes ----------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct GithubUser {
    pub id: i64,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubRepo {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub default_branch: Option<String>,
    pub private: bool,
    pub fork: bool,
    pub html_url: String,
    pub clone_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubPermission {
    /// "admin", "maintain", "write", "triage", "read", "none"
    pub permission: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubPullRequest {
    pub number: u64,
    pub html_url: String,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub scope: String,
    pub token_type: String,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponseRaw {
    access_token: Option<String>,
    refresh_token: Option<String>,
    scope: Option<String>,
    token_type: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}
