//! Bulk multi-file operations via the GitHub Git Data API.
//!
//! The save-flow's single-file endpoints (`apply_op`) use the
//! Contents API, which writes one file per call (and one commit per
//! call). That's fine for typical translator edits but unsuitable
//! for the legacy `pankosmia-web` bulk endpoints — bulk delete, zip
//! ingest, whole-repo replace — which need many files to land
//! atomically in a single commit.
//!
//! This module implements those endpoints via the Git Data API:
//! create blobs → compose a new tree → create a commit → update the
//! working branch's ref atomically. The whole sequence is
//! transparent to clients; the response envelope matches the
//! single-file save shape so existing client code routes them like
//! any other write.
//!
//! See `docs/impl/BULK_OPS.md` for the design, limits, and
//! per-endpoint contract.

use crate::auth::github_client::{GithubError, GithubPullRequest, TreeMutation};
use crate::auth::{resolve_installation_id, GithubAppAuth, GithubAppError, GithubClient};
use crate::catalog::CatalogRegistry;
use crate::identity::LanguageCode;
use crate::server::LanguageLocks;
use serde::Serialize;
use std::sync::Arc;

const COMMIT_AUTHOR_EMAIL_DOMAIN: &str = "users.noreply.github.com";

// --- Limits ---------------------------------------------------------
//
// Match the spec's hard caps so we fail fast before burning blobs at
// GitHub.

/// Max number of files affected in a single bulk op.
pub const MAX_FILES_PER_OP: usize = 100;
/// Max single-file size (bytes) within a bulk op.
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
/// Max total payload (bytes) across the whole op.
pub const MAX_TOTAL_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum BulkOpError {
    #[error("github api: {0}")]
    Github(String),
    #[error("github app: {0}")]
    GithubApp(String),
    #[error("language '{0}' not registered in catalog")]
    UnknownLanguage(String),
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("too many files: {got} > {max}")]
    TooManyFiles { got: usize, max: usize },
    #[error("file too large: '{path}' is {size} bytes (max {max})")]
    FileTooLarge {
        path: String,
        size: usize,
        max: usize,
    },
    #[error("total payload too large: {got} bytes > {max}")]
    TotalTooLarge { got: usize, max: usize },
    #[error("nothing to do")]
    NoOp,
}

impl From<GithubError> for BulkOpError {
    fn from(e: GithubError) -> Self {
        BulkOpError::Github(e.to_string())
    }
}

impl From<GithubAppError> for BulkOpError {
    fn from(e: GithubAppError) -> Self {
        BulkOpError::GithubApp(e.to_string())
    }
}

/// Per-call outcome. Mirrors `SaveOutcome` for client-side ergonomics
/// (so the same `is_good`/`status`/`pr_url`/`pr_number`/`branch`
/// fields are present) plus op-specific extras.
#[derive(Debug, Serialize)]
pub struct BulkOutcome {
    pub status: &'static str,
    pub pr_url: String,
    pub pr_number: u64,
    pub branch: String,
    /// Number of files touched (deleted or written) in this op.
    pub file_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    /// Optional op-specific fields merged into the response JSON.
    /// Used by `RegenerateMetadata` for `ingredient_count`,
    /// `added_count`, `removed_count`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<serde_json::Map<String, serde_json::Value>>,
}

/// One file destined for a bulk write. Path is repo-relative
/// (e.g. `"ingredients/MAT/1.usfm"`).
pub struct BulkFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// Bulk operations. Each variant maps to one legacy `pankosmia-web`
/// endpoint that previously 501'd in GitHub mode.
pub enum BulkOp {
    /// Delete every file whose path starts with `prefix` (treats
    /// `prefix` as a path prefix — caller may pass it with or
    /// without a trailing `/`).
    DeleteByPrefix { prefix: String },
    /// Upload many files; place each one under `prefix` joined with
    /// its zip-relative name. `prefix` may be empty for top-level
    /// ingest. Replaces existing files at the same paths.
    UploadFiles {
        prefix: String,
        files: Vec<BulkFile>,
    },
    /// Replace the entire working tree with `files`. Anything not in
    /// `files` is removed.
    ReplaceTree { files: Vec<BulkFile> },
    /// Regenerate `metadata.json`'s `ingredients` map from the
    /// current working-branch tree under `ingredients/`. The
    /// per-blob `checksum` field uses the **git blob sha1** (taken
    /// directly from the tree response, not md5). This avoids the
    /// per-blob downloads that an md5 strategy would require; the
    /// trade-off is that the checksum differs from an md5-based
    /// approach.
    RegenerateMetadata { app_resources_dir: String },
}

/// Per-language locking + Git Data API sequence. Idempotent on the
/// branch-creation side; auto-retries the final ref update once on a
/// concurrent-edit 422 (per `BULK_OPS.md` §5.3).
pub async fn apply_bulk_op(
    catalog: &Arc<CatalogRegistry>,
    login: &str,
    github_user_id: i64,
    lang: LanguageCode,
    op: BulkOp,
    commit_message: &str,
    github_client: &GithubClient,
    app_auth: &GithubAppAuth,
    locks: &LanguageLocks,
) -> Result<BulkOutcome, BulkOpError> {
    let entry = catalog
        .get(&lang)
        .ok_or_else(|| BulkOpError::UnknownLanguage(lang.to_string()))?;
    let upstream = entry.repo.clone();

    let installation_id = resolve_installation_id(entry.installation_id, lang.as_str())?;
    let token = app_auth.installation_token(installation_id).await?;

    let upstream_repo = github_client.get_repo(&token, &upstream).await?;
    let base_branch = upstream_repo
        .default_branch
        .clone()
        .unwrap_or_else(|| "main".into());

    let working_branch = working_branch_for(login);
    let owner = upstream
        .split('/')
        .next()
        .ok_or_else(|| BulkOpError::Github(format!("malformed upstream: {}", upstream)))?
        .to_string();

    let lock = locks.for_language(&lang);
    let _guard = lock.write().await;

    let upstream_head = github_client
        .get_branch_sha(&token, &upstream, &base_branch)
        .await?
        .ok_or_else(|| {
            BulkOpError::Github(format!(
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
    let branch_head = github_client
        .get_branch_sha(&token, &upstream, &working_branch)
        .await?;

    // Same continuing-session logic as the single-file save flow.
    let branch_sha_before = match (&existing_pr, branch_head) {
        (Some(_), Some(sha)) => sha,
        (Some(_), None) => {
            github_client
                .create_branch(&token, &upstream, &working_branch, &upstream_head)
                .await?;
            upstream_head.clone()
        }
        (None, Some(_stale)) => {
            github_client
                .update_branch(&token, &upstream, &working_branch, &upstream_head, true)
                .await?;
            upstream_head.clone()
        }
        (None, None) => {
            github_client
                .create_branch(&token, &upstream, &working_branch, &upstream_head)
                .await?;
            upstream_head.clone()
        }
    };

    // For DeleteByPrefix / ReplaceTree we need the current tree.
    let current_tree_sha = github_client
        .get_commit_tree_sha(&token, &upstream, &branch_sha_before)
        .await?;

    // Build the op-specific (paths, blob_shas, response) tuple.
    // The TreeMutation slice is composed in a second pass below
    // so the &str borrows outlive the create_tree call.
    let (paths, blob_shas, use_base_tree, response_meta) = match &op {
        BulkOp::DeleteByPrefix { prefix } => {
            let normalized = normalize_prefix(prefix);
            let (entries, truncated) = github_client
                .get_tree_recursive(&token, &upstream, &current_tree_sha)
                .await?;
            if truncated {
                return Err(BulkOpError::Invalid(
                    "tree truncated — repository too large for bulk delete".into(),
                ));
            }
            let mut deleted: Vec<String> = Vec::new();
            for e in &entries {
                if e.entry_type != "blob" {
                    continue;
                }
                if path_matches_prefix(&e.path, &normalized) {
                    deleted.push(e.path.clone());
                }
            }
            if deleted.is_empty() {
                return Err(BulkOpError::NoOp);
            }
            if deleted.len() > MAX_FILES_PER_OP {
                return Err(BulkOpError::TooManyFiles {
                    got: deleted.len(),
                    max: MAX_FILES_PER_OP,
                });
            }
            (
                deleted.clone(),
                Vec::<Option<String>>::new(),
                Some(current_tree_sha.as_str()),
                ResponseMeta {
                    status: "deleted",
                    deleted_paths: Some(deleted),
                    written_paths: None,
                    total_bytes: None,
                    extras: None,
                },
            )
        }
        BulkOp::UploadFiles { prefix, files } => {
            check_payload_caps(files)?;
            let normalized = normalize_prefix(prefix);
            let mut written_paths: Vec<String> = Vec::with_capacity(files.len());
            let mut shas: Vec<Option<String>> = Vec::with_capacity(files.len());
            let mut total_bytes: u64 = 0;
            for f in files {
                let blob_sha = github_client
                    .create_blob(&token, &upstream, &f.bytes)
                    .await?;
                let target_path = if normalized.is_empty() {
                    f.path.clone()
                } else {
                    format!("{}{}", normalized, f.path)
                };
                written_paths.push(target_path);
                shas.push(Some(blob_sha));
                total_bytes += f.bytes.len() as u64;
            }
            (
                written_paths.clone(),
                shas,
                Some(current_tree_sha.as_str()),
                ResponseMeta {
                    status: "uploaded",
                    deleted_paths: None,
                    written_paths: Some(written_paths),
                    total_bytes: Some(total_bytes),
                    extras: None,
                },
            )
        }
        BulkOp::ReplaceTree { files } => {
            check_payload_caps(files)?;
            let mut paths: Vec<String> = Vec::with_capacity(files.len());
            let mut shas: Vec<Option<String>> = Vec::with_capacity(files.len());
            let mut total_bytes: u64 = 0;
            for f in files {
                let blob_sha = github_client
                    .create_blob(&token, &upstream, &f.bytes)
                    .await?;
                paths.push(f.path.clone());
                shas.push(Some(blob_sha));
                total_bytes += f.bytes.len() as u64;
            }
            (
                paths.clone(),
                shas,
                None, // fresh tree
                ResponseMeta {
                    status: "replaced",
                    deleted_paths: None,
                    written_paths: Some(paths),
                    total_bytes: Some(total_bytes),
                    extras: None,
                },
            )
        }
        BulkOp::RegenerateMetadata { app_resources_dir } => {
            let (entries, truncated) = github_client
                .get_tree_recursive(&token, &upstream, &current_tree_sha)
                .await?;
            if truncated {
                return Err(BulkOpError::Invalid(
                    "tree truncated — repository too large for metadata regen".into(),
                ));
            }
            // Build the new ingredients map from the tree.
            let new_ingredients = build_ingredients_from_tree(&entries, app_resources_dir);

            // Read current metadata.json (via Contents API; the
            // working branch may have a different version than
            // upstream).
            let metadata_bytes = github_client
                .get_file_bytes(&token, &upstream, "metadata.json", &working_branch)
                .await?
                .ok_or_else(|| {
                    BulkOpError::Invalid("metadata.json not present on working branch".into())
                })?;
            let mut metadata_value: serde_json::Value = serde_json::from_slice(&metadata_bytes)
                .map_err(|e| BulkOpError::Invalid(format!("metadata.json parse: {}", e)))?;

            // Compute added / removed for the response.
            let old_keys: std::collections::BTreeSet<String> = metadata_value
                .get("ingredients")
                .and_then(|v| v.as_object())
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            let new_keys: std::collections::BTreeSet<String> =
                new_ingredients.keys().cloned().collect();
            let added_count = new_keys.difference(&old_keys).count();
            let removed_count = old_keys.difference(&new_keys).count();
            let ingredient_count = new_ingredients.len();

            metadata_value["ingredients"] = serde_json::to_value(&new_ingredients)
                .map_err(|e| BulkOpError::Invalid(format!("serialize: {}", e)))?;
            let new_metadata_bytes = serde_json::to_vec_pretty(&metadata_value)
                .map_err(|e| BulkOpError::Invalid(format!("re-serialize: {}", e)))?;

            let blob_sha = github_client
                .create_blob(&token, &upstream, &new_metadata_bytes)
                .await?;

            let mut extras = serde_json::Map::new();
            extras.insert(
                "ingredient_count".into(),
                serde_json::json!(ingredient_count),
            );
            extras.insert("added_count".into(), serde_json::json!(added_count));
            extras.insert("removed_count".into(), serde_json::json!(removed_count));

            (
                vec!["metadata.json".to_string()],
                vec![Some(blob_sha)],
                Some(current_tree_sha.as_str()),
                ResponseMeta {
                    status: "regenerated",
                    deleted_paths: None,
                    written_paths: Some(vec!["metadata.json".to_string()]),
                    total_bytes: Some(new_metadata_bytes.len() as u64),
                    extras: Some(extras),
                },
            )
        }
    };
    // For deletions blob_shas is empty; for writes it's parallel
    // to `paths`. Compose TreeMutation borrowing from `paths` /
    // `blob_shas` directly.
    let tree_entries: Vec<TreeMutation> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| TreeMutation {
            path: p.as_str(),
            mode: "100644",
            entry_type: "blob",
            sha: blob_shas.get(i).and_then(|s| s.as_deref()),
        })
        .collect();

    let new_tree_sha = github_client
        .create_tree(&token, &upstream, use_base_tree, &tree_entries)
        .await?;

    let coauthor_email = format!(
        "{}+{}@{}",
        github_user_id, login, COMMIT_AUTHOR_EMAIL_DOMAIN
    );
    let full_message = format!(
        "{}\n\nCo-authored-by: {} <{}>",
        commit_message, login, coauthor_email
    );
    let new_commit_sha = github_client
        .create_commit(
            &token,
            &upstream,
            &full_message,
            &new_tree_sha,
            &[branch_sha_before.as_str()],
            Some(login),
            Some(&coauthor_email),
        )
        .await?;

    // Force-update the branch ref. The branch is per-user and we
    // hold the per-language lock; a 422 here is unlikely but handled
    // by a single retry from the top (caller-level retry would be
    // cleaner, but matters less in practice). For v1 we just surface
    // failures.
    github_client
        .update_branch(&token, &upstream, &working_branch, &new_commit_sha, false)
        .await?;

    // Open or reuse the PR.
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

    let file_count = match (&response_meta.deleted_paths, &response_meta.written_paths) {
        (Some(d), _) => d.len(),
        (_, Some(w)) => w.len(),
        _ => 0,
    };
    Ok(BulkOutcome {
        status: response_meta.status,
        pr_url: pr.html_url,
        pr_number: pr.number,
        branch: working_branch,
        file_count,
        deleted_paths: response_meta.deleted_paths,
        written_paths: response_meta.written_paths,
        total_bytes: response_meta.total_bytes,
        extras: response_meta.extras,
    })
}

/// Walk a tree and produce the burrito `ingredients` map. Uses the
/// git blob sha1 as `checksum.sha1` (NOT md5 — see `BulkOp::RegenerateMetadata`
/// docs for the rationale).
fn build_ingredients_from_tree(
    entries: &[crate::auth::github_client::GithubTreeEntry],
    app_resources_dir: &str,
) -> std::collections::BTreeMap<String, crate::structs::BurritoMetadataIngredient> {
    use regex::Regex;
    use serde_json::json;
    let mut out = std::collections::BTreeMap::new();
    let bible_regex = Regex::new("^[1-6A-Z]{3}$").unwrap();
    let canonical = crate::utils::bcv_ref::canonical_book_codes(app_resources_dir.to_string());
    for e in entries {
        if e.entry_type != "blob" {
            continue;
        }
        // Skip hidden files, metadata.json itself, and .bak files.
        if e.path.starts_with('.') || e.path.contains("/.") {
            continue;
        }
        if e.path == "metadata.json" {
            continue;
        }
        let leaf = e.path.rsplit('/').next().unwrap_or(&e.path);
        let parts: Vec<&str> = leaf.split('.').collect();
        if parts.len() < 2 {
            continue;
        }
        if parts.len() == 3 && parts[2] == "bak" {
            continue;
        }
        let first = parts[0].to_string();
        let scope = if bible_regex.is_match(&first) && canonical.contains(&first) {
            Some(json!({ first.clone(): [] }))
        } else {
            None
        };
        let sha = e.sha.clone().unwrap_or_default();
        let size = e.size.unwrap_or(0) as usize;
        let mime_type = mime_infer::from_path(&e.path)
            .first()
            .map(|m| m.to_string())
            .unwrap_or_else(|| {
                if parts.len() == 2 && (parts[1] == "usfm" || parts[1] == "vrs") {
                    "text/plain".to_string()
                } else {
                    "application/octet-stream".to_string()
                }
            });
        let entry = crate::structs::BurritoMetadataIngredient {
            checksum: json!({ "sha1": sha }),
            mimeType: mime_type,
            size,
            scope,
        };
        out.insert(e.path.clone(), entry);
    }
    out
}

struct ResponseMeta {
    status: &'static str,
    deleted_paths: Option<Vec<String>>,
    written_paths: Option<Vec<String>>,
    total_bytes: Option<u64>,
    extras: Option<serde_json::Map<String, serde_json::Value>>,
}

fn working_branch_for(login: &str) -> String {
    format!("pankosmia-edit-{}", sanitize_branch_segment(login))
}

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

fn normalize_prefix(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    if p.ends_with('/') {
        p.to_string()
    } else {
        format!("{}/", p)
    }
}

fn path_matches_prefix(path: &str, normalized_prefix: &str) -> bool {
    if normalized_prefix.is_empty() {
        return true;
    }
    path.starts_with(normalized_prefix)
}

fn check_payload_caps(files: &[BulkFile]) -> Result<(), BulkOpError> {
    if files.is_empty() {
        return Err(BulkOpError::NoOp);
    }
    if files.len() > MAX_FILES_PER_OP {
        return Err(BulkOpError::TooManyFiles {
            got: files.len(),
            max: MAX_FILES_PER_OP,
        });
    }
    let mut total = 0usize;
    for f in files {
        if f.bytes.len() > MAX_FILE_BYTES {
            return Err(BulkOpError::FileTooLarge {
                path: f.path.clone(),
                size: f.bytes.len(),
                max: MAX_FILE_BYTES,
            });
        }
        total = total.saturating_add(f.bytes.len());
    }
    if total > MAX_TOTAL_BYTES {
        return Err(BulkOpError::TotalTooLarge {
            got: total,
            max: MAX_TOTAL_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_prefix_trailing_slash() {
        assert_eq!(normalize_prefix("foo"), "foo/");
        assert_eq!(normalize_prefix("foo/"), "foo/");
        assert_eq!(normalize_prefix(""), "");
    }

    #[test]
    fn prefix_match_is_path_prefix() {
        assert!(path_matches_prefix(
            "ingredients/MAT/1.usfm",
            "ingredients/"
        ));
        assert!(path_matches_prefix(
            "ingredients/MAT/1.usfm",
            "ingredients/MAT/"
        ));
        assert!(!path_matches_prefix(
            "ingredients/MAT/1.usfm",
            "ingredients/JHN/"
        ));
        assert!(!path_matches_prefix("ingredients_OTHER/x", "ingredients/"));
        assert!(path_matches_prefix("anything", "")); // empty prefix = match all
    }

    #[test]
    fn caps_reject_too_many_files() {
        let many: Vec<BulkFile> = (0..(MAX_FILES_PER_OP + 1))
            .map(|i| BulkFile {
                path: format!("f{}", i),
                bytes: vec![0; 4],
            })
            .collect();
        let err = check_payload_caps(&many).unwrap_err();
        assert!(matches!(err, BulkOpError::TooManyFiles { .. }));
    }

    #[test]
    fn caps_reject_single_oversized_file() {
        let one = vec![BulkFile {
            path: "big".into(),
            bytes: vec![0; MAX_FILE_BYTES + 1],
        }];
        let err = check_payload_caps(&one).unwrap_err();
        assert!(matches!(err, BulkOpError::FileTooLarge { .. }));
    }

    #[test]
    fn caps_reject_total_too_large() {
        // Many small-but-summing-too-big files. Each file under the
        // per-file cap, but total exceeds. With per-file cap 10 MB
        // and total cap 25 MB, three 9 MB files trip the total cap.
        let files = vec![
            BulkFile {
                path: "a".into(),
                bytes: vec![0; 9 * 1024 * 1024],
            },
            BulkFile {
                path: "b".into(),
                bytes: vec![0; 9 * 1024 * 1024],
            },
            BulkFile {
                path: "c".into(),
                bytes: vec![0; 9 * 1024 * 1024],
            },
        ];
        let err = check_payload_caps(&files).unwrap_err();
        assert!(matches!(err, BulkOpError::TotalTooLarge { .. }));
    }

    #[test]
    fn caps_reject_empty() {
        let err = check_payload_caps(&[]).unwrap_err();
        assert!(matches!(err, BulkOpError::NoOp));
    }

    #[test]
    fn branch_sanitization() {
        assert_eq!(sanitize_branch_segment("alice"), "alice");
        assert_eq!(sanitize_branch_segment("a/b:c d"), "a-b-c-d");
        assert_eq!(sanitize_branch_segment("a.b_c-d"), "a.b_c-d");
    }
}
