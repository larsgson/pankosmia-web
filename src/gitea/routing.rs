use crate::gitea::config::CuratedOrgs;
use std::path::Path;

pub struct ParsedRepoPath {
    pub server: String,
    pub org: String,
    pub repo: String,
}

pub enum ReadSource {
    Gitea(ParsedRepoPath),
    LocalFilesystem,
}

pub fn resolve_read_source(curated: &CuratedOrgs, repo_path: &Path) -> ReadSource {
    let mut components = repo_path.components();
    let server = match components.next() {
        Some(c) => c.as_os_str().to_string_lossy().to_string(),
        None => return ReadSource::LocalFilesystem,
    };
    let org = match components.next() {
        Some(c) => c.as_os_str().to_string_lossy().to_string(),
        None => return ReadSource::LocalFilesystem,
    };
    let repo = match components.next() {
        Some(c) => c.as_os_str().to_string_lossy().to_string(),
        None => return ReadSource::LocalFilesystem,
    };
    let key = format!("{}/{}", server, org);
    if curated.is_curated(&key) {
        ReadSource::Gitea(ParsedRepoPath { server, org, repo })
    } else {
        ReadSource::LocalFilesystem
    }
}
