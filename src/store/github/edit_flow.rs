//! GitHub edit flow (App-model).
//!
//! Per save:
//!   1. Resolve the language's upstream repo from the catalog.
//!   2. Resolve the installation ID (per-language override → global
//!      `PANKOSMIA_DEFAULT_INSTALLATION_ID`).
//!   3. Mint an installation token via `GithubAppAuth`.
//!   4. Read `refs/heads/<default-branch>` to find upstream HEAD SHA.
//!   5. Working branch: `pankosmia-edit-<user-login>`. Look up the
//!      user's existing open PR on that branch.
//!        - If an open PR exists, the branch is left as-is and the
//!          save appends a commit on top (audit-trail style).
//!        - Otherwise the branch is reset/created at upstream HEAD
//!          so a stale-from-merged-session branch doesn't
//!          accidentally re-PR old work.
//!   6. Fetch the existing blob SHA at the file path on the working
//!      branch.
//!   7. `PUT /repos/{repo}/contents/{path}` with new bytes, the
//!      blob SHA (if any), and a commit message carrying a
//!      `Co-authored-by: <user-login>` trailer for attribution.
//!   8. If no open PR was found in step 5, open one for the branch.
//!
//! No git2, no per-user disk state, no forks. Per-language locks
//! serialize concurrent saves on the same language so we don't race
//! on the PR-lookup → branch-state → PUT sequence.
//!
//! Note: a previous draft of this module reset the working branch to
//! upstream HEAD on every save. That triggered GitHub's auto-close
//! behaviour: the brief no-diff window between reset and PUT closed
//! any open PR for the branch, causing each save to mint a fresh PR.
//! `internal-docs/AUTH_MODEL.md` §7 records the resulting decision (commits
//! accumulate within a session, reviewer can squash at merge).

use crate::auth::github_client::{GithubError, GithubPullRequest};
use crate::auth::{resolve_installation_id, GithubAppAuth, GithubAppError, GithubClient};
use crate::catalog::CatalogRegistry;
use crate::identity::LanguageCode;
use crate::server::LanguageLocks;
use serde::Serialize;
use std::sync::Arc;

const COMMIT_AUTHOR_EMAIL_DOMAIN: &str = "users.noreply.github.com";

#[derive(Debug, thiserror::Error)]
pub enum EditFlowError {
    #[error("github api: {0}")]
    Github(String),
    #[error("github app: {0}")]
    GithubApp(String),
    #[error("language '{0}' not registered in catalog")]
    UnknownLanguage(String),
}

impl From<GithubError> for EditFlowError {
    fn from(e: GithubError) -> Self {
        EditFlowError::Github(e.to_string())
    }
}

impl From<GithubAppError> for EditFlowError {
    fn from(e: GithubAppError) -> Self {
        EditFlowError::GithubApp(e.to_string())
    }
}

#[derive(Debug, Serialize)]
pub struct SaveOutcome {
    pub status: &'static str,
    pub pr_url: String,
    pub pr_number: u64,
    pub branch: String,
}

pub struct GithubEditFlow {
    registry: Arc<CatalogRegistry>,
}

impl GithubEditFlow {
    pub fn new(registry: Arc<CatalogRegistry>) -> Self {
        Self { registry }
    }

    /// Save → ensure-branch → PUT-contents → PR. Returns the PR's
    /// URL and number.
    pub async fn save_ingredient(
        &self,
        login: &str,
        github_user_id: i64,
        lang: LanguageCode,
        ipath: &str,
        bytes: &[u8],
        commit_message: &str,
        github_client: &GithubClient,
        app_auth: &GithubAppAuth,
        locks: &LanguageLocks,
    ) -> Result<SaveOutcome, EditFlowError> {
        let entry = self
            .registry
            .get(&lang)
            .ok_or_else(|| EditFlowError::UnknownLanguage(lang.to_string()))?;
        let upstream = entry.repo.clone();

        let installation_id = resolve_installation_id(entry.installation_id, lang.as_str())?;
        let token = app_auth.installation_token(installation_id).await?;

        // Look up the upstream default branch. Catalog doesn't carry
        // this, so we ask GitHub once per save. (Cheap; could be
        // cached in the catalog entry later.)
        let upstream_repo = github_client.get_repo(&token, &upstream).await?;
        let base_branch = upstream_repo
            .default_branch
            .clone()
            .unwrap_or_else(|| "main".into());

        let working_branch = format!("pankosmia-edit-{}", sanitize_branch_segment(login));
        let owner = upstream
            .split('/')
            .next()
            .ok_or_else(|| EditFlowError::Github(format!("malformed upstream: {}", upstream)))?
            .to_string();

        // Serialise concurrent saves on the same language so we
        // don't race on the PR-lookup → branch-state → PUT sequence.
        let lock = locks.for_language(&lang);
        let _guard = lock.write().await;

        let upstream_head = github_client
            .get_branch_sha(&token, &upstream, &base_branch)
            .await?
            .ok_or_else(|| {
                EditFlowError::Github(format!(
                    "upstream '{}' has no '{}' branch — repo empty?",
                    upstream, base_branch
                ))
            })?;

        // Look up the user's existing open PR for this branch BEFORE
        // touching the branch. If one exists we're continuing a
        // session and the branch must be left alone (force-resetting
        // would make GitHub auto-close the PR during the brief
        // no-diff window).
        let head_query = format!("{}:{}", owner, working_branch);
        let existing_pr: Option<GithubPullRequest> = github_client
            .list_pulls(&token, &upstream, Some(&head_query), Some(&base_branch), "open")
            .await?
            .into_iter()
            .next();
        let branch_exists = github_client
            .get_branch_sha(&token, &upstream, &working_branch)
            .await?
            .is_some();

        match (&existing_pr, branch_exists) {
            (Some(_), true) => {
                // Continuing a live session — leave the branch alone.
            }
            (Some(_), false) => {
                // PR open but branch missing. Shouldn't normally
                // happen; recreate the branch and the next save
                // commit lands on it.
                github_client
                    .create_branch(&token, &upstream, &working_branch, &upstream_head)
                    .await?;
            }
            (None, true) => {
                // Stale branch from a merged or manually-closed
                // previous session — reset to upstream HEAD so we
                // don't accumulate on top of merged history.
                github_client
                    .update_branch(&token, &upstream, &working_branch, &upstream_head, true)
                    .await?;
            }
            (None, false) => {
                // Fresh start.
                github_client
                    .create_branch(&token, &upstream, &working_branch, &upstream_head)
                    .await?;
            }
        }

        // PUT the file. blob SHA reflects whatever's currently on the
        // working branch — for continuing sessions that's the user's
        // most recent commit on this file; for fresh branches it
        // matches upstream HEAD (or None if the file is new).
        let blob_sha = github_client
            .get_file_blob_sha(&token, &upstream, &content_path(ipath), &working_branch)
            .await?;
        let coauthor_email =
            format!("{}+{}@{}", github_user_id, login, COMMIT_AUTHOR_EMAIL_DOMAIN);
        let full_message = format!(
            "{}\n\nCo-authored-by: {} <{}>",
            commit_message, login, coauthor_email
        );
        github_client
            .put_file_contents(
                &token,
                &upstream,
                &content_path(ipath),
                &working_branch,
                bytes,
                &full_message,
                blob_sha.as_deref(),
            )
            .await?;

        // Reuse the open PR or open a new one (we already looked up
        // open PRs at the top of the function).
        let pr: GithubPullRequest = if let Some(p) = existing_pr {
            p
        } else {
            let title = format!("pankosmia: edits to {}", lang.as_str());
            let body = format!(
                "Edits submitted via pankosmia-docker on behalf of @{}.\n\n\
                 Working branch: `{}`.",
                login, working_branch
            );
            github_client
                .open_pull_request(
                    &token,
                    &upstream,
                    &title,
                    &head_query,
                    &base_branch,
                    &body,
                )
                .await?
        };
        Ok(SaveOutcome {
            status: "saved",
            pr_url: pr.html_url,
            pr_number: pr.number,
            branch: working_branch,
        })
    }
}

fn content_path(ipath: &str) -> String {
    format!("ingredients/{}", ipath)
}

/// Strip characters that aren't safe in a git ref. GitHub usernames
/// can contain hyphens and digits (already valid) but not `/`, `:`,
/// or whitespace — none of which actually appear in real logins, but
/// be defensive in case a future identity provider supplies looser
/// values.
fn sanitize_branch_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}
