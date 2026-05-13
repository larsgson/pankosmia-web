//! GitHub edit flow (App-model).
//!
//! Each save endpoint dispatches into `apply_op` with a `SaveOp`
//! describing what to do on the working branch. Per call:
//!
//!   1. Resolve the language's upstream repo from the catalog.
//!   2. Resolve the installation ID (per-language override → global
//!      `PANKOSMIA_DEFAULT_INSTALLATION_ID`).
//!   3. Mint an installation token via `GithubAppAuth`.
//!   4. Read `refs/heads/<default-branch>` to find upstream HEAD SHA.
//!   5. Working branch: `pankosmia-edit-<user-login>`. Look up the
//!      user's existing open PR on that branch.
//!        - If an open PR exists, the branch is left as-is and the
//!          op appends a commit on top (audit-trail style).
//!        - Otherwise the branch is reset/created at upstream HEAD
//!          so a stale-from-merged-session branch doesn't
//!          accidentally re-PR old work.
//!   6. Apply the op via the Contents API (PUT/DELETE), with a
//!      commit message carrying a `Co-authored-by: <user-login>`
//!      trailer for attribution.
//!   7. If no open PR was found in step 5, open one for the branch.
//!
//! No git2, no per-user disk state, no forks. Per-language locks
//! serialize concurrent ops on the same language so we don't race
//! on the PR-lookup → branch-state → write sequence.
//!
//! Note: a previous draft reset the working branch to upstream HEAD
//! on every save, which triggered GitHub's PR auto-close behaviour
//! during the brief no-diff window. `internal-docs/AUTH_MODEL.md` §7
//! records the resulting decision (commits accumulate within a
//! session, reviewer can squash at merge).

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
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid argument: {0}")]
    Invalid(String),
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

/// What to do on the working branch in this call. Each variant maps
/// to one or two Contents-API operations.
#[derive(Debug)]
pub enum SaveOp<'a> {
    /// Create or replace one ingredient file.
    Put { ipath: &'a str, bytes: &'a [u8] },
    /// Delete one ingredient file from the working branch.
    Delete { ipath: &'a str },
    /// Copy `src_ipath` to `target_ipath` (read source bytes from
    /// working branch, write to target). Optionally remove the
    /// source afterwards.
    Copy {
        src_ipath: &'a str,
        target_ipath: &'a str,
        delete_src: bool,
    },
    /// Revert an ingredient to whatever upstream HEAD says, undoing
    /// any working-branch changes to that file. If the file doesn't
    /// exist at upstream HEAD, removes it from the working branch.
    Revert { ipath: &'a str },
}

pub struct GithubEditFlow {
    registry: Arc<CatalogRegistry>,
}

impl GithubEditFlow {
    pub fn new(registry: Arc<CatalogRegistry>) -> Self {
        Self { registry }
    }

    /// Apply a `SaveOp` to the user's working branch, ensuring the
    /// branch state and PR are consistent. Returns the PR's URL and
    /// number.
    pub async fn apply_op(
        &self,
        login: &str,
        github_user_id: i64,
        lang: LanguageCode,
        op: SaveOp<'_>,
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

        // Serialise concurrent ops on the same language.
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

        let head_query = format!("{}:{}", owner, working_branch);
        let existing_pr: Option<GithubPullRequest> = github_client
            .list_pulls(
                &token,
                &upstream,
                Some(&head_query),
                Some(&base_branch),
                "open",
            )
            .await?
            .into_iter()
            .next();
        let branch_exists = github_client
            .get_branch_sha(&token, &upstream, &working_branch)
            .await?
            .is_some();

        match (&existing_pr, branch_exists) {
            (Some(_), true) => { /* continuing session */ }
            (Some(_), false) => {
                github_client
                    .create_branch(&token, &upstream, &working_branch, &upstream_head)
                    .await?;
            }
            (None, true) => {
                github_client
                    .update_branch(&token, &upstream, &working_branch, &upstream_head, true)
                    .await?;
            }
            (None, false) => {
                github_client
                    .create_branch(&token, &upstream, &working_branch, &upstream_head)
                    .await?;
            }
        }

        let coauthor_email = format!(
            "{}+{}@{}",
            github_user_id, login, COMMIT_AUTHOR_EMAIL_DOMAIN
        );
        let full_message = format!(
            "{}\n\nCo-authored-by: {} <{}>",
            commit_message, login, coauthor_email
        );

        match op {
            SaveOp::Put { ipath, bytes } => {
                let path = content_path(ipath);
                let blob_sha = github_client
                    .get_file_blob_sha(&token, &upstream, &path, &working_branch)
                    .await?;
                github_client
                    .put_file_contents(
                        &token,
                        &upstream,
                        &path,
                        &working_branch,
                        bytes,
                        &full_message,
                        blob_sha.as_deref(),
                    )
                    .await?;
            }
            SaveOp::Delete { ipath } => {
                let path = content_path(ipath);
                let blob_sha = github_client
                    .get_file_blob_sha(&token, &upstream, &path, &working_branch)
                    .await?
                    .ok_or_else(|| {
                        EditFlowError::NotFound(format!("ingredient '{}' not on branch", ipath))
                    })?;
                github_client
                    .delete_file_contents(
                        &token,
                        &upstream,
                        &path,
                        &working_branch,
                        &full_message,
                        &blob_sha,
                    )
                    .await?;
            }
            SaveOp::Copy {
                src_ipath,
                target_ipath,
                delete_src,
            } => {
                if src_ipath == target_ipath {
                    return Err(EditFlowError::Invalid(
                        "src and target must be different".into(),
                    ));
                }
                let src_path = content_path(src_ipath);
                let target_path = content_path(target_ipath);
                let src_bytes = github_client
                    .get_file_bytes(&token, &upstream, &src_path, &working_branch)
                    .await?
                    .ok_or_else(|| {
                        EditFlowError::NotFound(format!("source '{}' not on branch", src_ipath))
                    })?;
                let target_blob_sha = github_client
                    .get_file_blob_sha(&token, &upstream, &target_path, &working_branch)
                    .await?;
                github_client
                    .put_file_contents(
                        &token,
                        &upstream,
                        &target_path,
                        &working_branch,
                        &src_bytes,
                        &full_message,
                        target_blob_sha.as_deref(),
                    )
                    .await?;
                if delete_src {
                    // Re-read source blob SHA — the PUT above may have
                    // changed branch HEAD, so a fresh lookup is safest.
                    if let Some(src_sha) = github_client
                        .get_file_blob_sha(&token, &upstream, &src_path, &working_branch)
                        .await?
                    {
                        let delete_msg = format!("{} (delete src)", full_message);
                        github_client
                            .delete_file_contents(
                                &token,
                                &upstream,
                                &src_path,
                                &working_branch,
                                &delete_msg,
                                &src_sha,
                            )
                            .await?;
                    }
                }
            }
            SaveOp::Revert { ipath } => {
                let path = content_path(ipath);
                let upstream_bytes = github_client
                    .get_file_bytes(&token, &upstream, &path, &base_branch)
                    .await?;
                let branch_blob_sha = github_client
                    .get_file_blob_sha(&token, &upstream, &path, &working_branch)
                    .await?;
                match (upstream_bytes, branch_blob_sha) {
                    (Some(bytes), maybe_sha) => {
                        github_client
                            .put_file_contents(
                                &token,
                                &upstream,
                                &path,
                                &working_branch,
                                &bytes,
                                &full_message,
                                maybe_sha.as_deref(),
                            )
                            .await?;
                    }
                    (None, Some(sha)) => {
                        github_client
                            .delete_file_contents(
                                &token,
                                &upstream,
                                &path,
                                &working_branch,
                                &full_message,
                                &sha,
                            )
                            .await?;
                    }
                    (None, None) => {
                        return Err(EditFlowError::NotFound(format!(
                            "ingredient '{}' not present at upstream or on branch",
                            ipath
                        )));
                    }
                }
            }
        }

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
                .open_pull_request(&token, &upstream, &title, &head_query, &base_branch, &body)
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
