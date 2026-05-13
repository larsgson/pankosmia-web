//! Path resolution + traversal validation for the FS backend.
//!
//! Layout under `<workspace_root>/`:
//!
//! ```text
//! <workspace_root>/
//! ├── .pankosmia/                ← reserved Pankosmia internal storage
//! │   │                            (leading `.` makes legacy
//! │   │                            `list_local_repos` skip it)
//! │   ├── users/<user_id>/
//! │   │   ├── settings.json      # UserSettings
//! │   │   ├── auth_tokens.json   # gitea OAuth tokens (per-user)
//! │   │   └── auth_requests.json # short-lived OAuth state
//! │   └── languages/<lang>/
//! │       ├── members.json       # LanguageMembership rows
//! │       ├── app_state.json     # AppState (per-language)
//! │       ├── bcv/<user_id>.json # Bcv (per-user-per-language)
//! │       └── repos.json         # RepoRecord registry
//! └── <source>/<org>/<name>/     ← legacy repo working trees,
//!     └── (.git, ingredients,      unchanged through M3
//!          metadata.json, ...)
//! ```
//!
//! The `.pankosmia/` prefix isolates Pankosmia-managed metadata from
//! user-owned repo directories. Legitimate "source" names like
//! `_local_` (single-underscore) are not affected.

use crate::identity::{LanguageCode, RepoId, UserId};
use crate::store::types::{StoreError, StoreResult};
use std::path::{Path, PathBuf};

/// Reserved top-level directory inside `<workspace_root>/` that holds
/// Pankosmia internal state. Legacy `list_local_repos` skips dot-
/// prefixed entries, so this name is naturally invisible to the
/// repo-enumeration code.
pub const RESERVED_PREFIX: &str = ".pankosmia";

fn pankosmia_root(root: &Path) -> PathBuf {
    root.join(RESERVED_PREFIX)
}

fn languages_root(root: &Path) -> PathBuf {
    pankosmia_root(root).join("languages")
}

fn users_root(root: &Path) -> PathBuf {
    pankosmia_root(root).join("users")
}

/// Reject any user-supplied path segment that could escape the
/// workspace subtree (path traversal) or otherwise break filesystem
/// assumptions. Centralized here so endpoints — and future trait
/// methods — never have to remember to revalidate.
pub fn validate_segment(s: &str) -> StoreResult<()> {
    if s.is_empty() {
        return Err(StoreError::Invalid("empty segment".into()));
    }
    if s == "." || s == ".." {
        return Err(StoreError::Invalid("relative segment".into()));
    }
    if s.contains('\0') || s.contains('/') || s.contains('\\') {
        return Err(StoreError::Invalid("forbidden character in segment".into()));
    }
    if s.starts_with('/') || s.starts_with('\\') {
        return Err(StoreError::Invalid("absolute segment".into()));
    }
    Ok(())
}

pub fn language_dir(root: &Path, lang: &LanguageCode) -> PathBuf {
    languages_root(root).join(lang.as_str())
}

pub fn members_file(root: &Path, lang: &LanguageCode) -> PathBuf {
    language_dir(root, lang).join("members.json")
}

pub fn app_state_file(root: &Path, lang: &LanguageCode) -> PathBuf {
    language_dir(root, lang).join("app_state.json")
}

pub fn bcv_file(root: &Path, lang: &LanguageCode, user: UserId) -> PathBuf {
    language_dir(root, lang)
        .join("bcv")
        .join(format!("{}.json", user))
}

pub fn repo_registry_file(root: &Path, lang: &LanguageCode) -> PathBuf {
    language_dir(root, lang).join("repos.json")
}

/// UUID-keyed working tree path (Phase 2 layout). For Phase 1 the
/// legacy path-based scheme is used instead — see
/// `legacy_repo_workspace_path` below.
pub fn repo_dir(root: &Path, lang: &LanguageCode, repo: RepoId) -> PathBuf {
    language_dir(root, lang).join(repo.to_string())
}

pub fn user_dir(root: &Path, user: UserId) -> PathBuf {
    users_root(root).join(user.to_string())
}

pub fn user_settings_file(root: &Path, user: UserId) -> PathBuf {
    user_dir(root, user).join("settings.json")
}

pub fn user_auth_tokens_file(root: &Path, user: UserId) -> PathBuf {
    user_dir(root, user).join("auth_tokens.json")
}

pub fn user_auth_requests_file(root: &Path, user: UserId) -> PathBuf {
    user_dir(root, user).join("auth_requests.json")
}

/// Resolve a legacy `<source>/<org>/<name>` repo path to an absolute
/// path under `workspace_root`. Validates each segment against the
/// path-traversal rules and against the reserved internal prefix.
pub fn legacy_repo_workspace_path(root: &Path, repo_path: &Path) -> StoreResult<PathBuf> {
    let mut out = root.to_path_buf();
    let mut count = 0;
    for component in repo_path.components() {
        match component {
            std::path::Component::Normal(s) => {
                let seg = s.to_string_lossy();
                validate_segment(&seg)?;
                if count == 0 && seg == RESERVED_PREFIX {
                    return Err(StoreError::Invalid(format!(
                        "{} is a reserved top-level name",
                        RESERVED_PREFIX
                    )));
                }
                out.push(seg.as_ref());
                count += 1;
            }
            _ => return Err(StoreError::Invalid("non-normal path component".into())),
        }
    }
    if count == 0 {
        return Err(StoreError::Invalid("empty repo path".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_segment_rejects_traversal_payloads() {
        for bad in [
            "..",
            ".",
            "/etc/passwd",
            "..\\..\\windows",
            "foo/bar",
            "foo\\bar",
            "foo\0bar",
            "/leading",
            "\\leading",
        ] {
            assert!(
                validate_segment(bad).is_err(),
                "expected {:?} to be rejected",
                bad
            );
        }
    }

    #[test]
    fn validate_segment_accepts_benign_inputs() {
        for ok in ["hello", "main", "_local_", "a.b.c", "fr-CA", "01.tsv"] {
            assert!(
                validate_segment(ok).is_ok(),
                "expected {:?} to be accepted",
                ok
            );
        }
    }

    #[test]
    fn validate_segment_rejects_empty() {
        assert!(validate_segment("").is_err());
    }

    #[test]
    fn legacy_repo_workspace_path_resolves_normal_path() {
        let root = std::path::PathBuf::from("/tmp/test");
        let p = legacy_repo_workspace_path(&root, std::path::Path::new("_local_/_local_/myrepo"))
            .unwrap();
        assert_eq!(
            p,
            std::path::PathBuf::from("/tmp/test/_local_/_local_/myrepo")
        );
    }

    #[test]
    fn legacy_repo_workspace_path_rejects_traversal() {
        let root = std::path::PathBuf::from("/tmp/test");
        assert!(legacy_repo_workspace_path(&root, std::path::Path::new("../escape")).is_err());
        assert!(legacy_repo_workspace_path(&root, std::path::Path::new("a/../b")).is_err());
        assert!(legacy_repo_workspace_path(&root, std::path::Path::new("/abs/path")).is_err());
        assert!(legacy_repo_workspace_path(&root, std::path::Path::new("")).is_err());
    }

    #[test]
    fn legacy_repo_workspace_path_rejects_reserved_prefix() {
        let root = std::path::PathBuf::from("/tmp/test");
        assert!(
            legacy_repo_workspace_path(&root, std::path::Path::new(".pankosmia/sneak")).is_err()
        );
    }
}
