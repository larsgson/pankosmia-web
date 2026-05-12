//! Schema and parser for `languages.yaml`.
//!
//! The on-disk shape is documented in
//! `docs/STRATEGY_GITHUB_BACKEND.md` §4.2. This module parses it
//! and validates the same rules the catalog repo's CI validator
//! enforces — so an invalid catalog is caught both upstream (at PR
//! time, by the GitHub Action) and at server startup (here).

use crate::identity::LanguageCode;
use serde::{Deserialize, Serialize};

const SUPPORTED_SCHEMA_VERSIONS: &[u32] = &[1];

#[derive(Debug, Deserialize)]
pub struct CatalogFile {
    pub schema_version: u32,
    pub languages: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogEntry {
    pub code: String,
    pub display_name: String,
    pub repo: String,
    pub script: Option<String>,
    pub direction: Option<String>,
    pub added_at: Option<String>,
    pub added_by: Option<String>,
    pub notes: Option<String>,
    /// Optional per-language GitHub App installation override. If
    /// absent, the resolver falls back to the global
    /// `PANKOSMIA_DEFAULT_INSTALLATION_ID` env var. See
    /// `internal-docs/AUTH_MODEL.md` §7.
    pub installation_id: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogParseError {
    #[error("yaml: {0}")]
    Yaml(String),
    #[error("unsupported schema_version {0}")]
    UnsupportedSchema(u32),
    #[error("entry[{index}]: {reason}")]
    BadEntry { index: usize, reason: String },
    #[error("duplicate code: {0}")]
    DuplicateCode(String),
    #[error("duplicate repo: {0}")]
    DuplicateRepo(String),
}

impl CatalogFile {
    pub fn parse_yaml(s: &str) -> Result<Self, CatalogParseError> {
        let raw: CatalogFile =
            serde_yaml::from_str(s).map_err(|e| CatalogParseError::Yaml(e.to_string()))?;
        raw.validate()?;
        Ok(raw)
    }

    fn validate(&self) -> Result<(), CatalogParseError> {
        if !SUPPORTED_SCHEMA_VERSIONS.contains(&self.schema_version) {
            return Err(CatalogParseError::UnsupportedSchema(self.schema_version));
        }
        let mut seen_codes = std::collections::HashSet::new();
        let mut seen_repos = std::collections::HashSet::new();
        for (i, entry) in self.languages.iter().enumerate() {
            // BCP 47 subset — same rules as `LanguageCode::parse`.
            LanguageCode::parse(&entry.code).map_err(|e| CatalogParseError::BadEntry {
                index: i,
                reason: format!("invalid code: {}", e),
            })?;
            // owner/name shape.
            let parts: Vec<&str> = entry.repo.split('/').collect();
            if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err(CatalogParseError::BadEntry {
                    index: i,
                    reason: format!("repo must be owner/name, got {:?}", entry.repo),
                });
            }
            if entry.display_name.is_empty() {
                return Err(CatalogParseError::BadEntry {
                    index: i,
                    reason: "display_name is empty".into(),
                });
            }
            if let Some(d) = &entry.direction {
                if d != "ltr" && d != "rtl" {
                    return Err(CatalogParseError::BadEntry {
                        index: i,
                        reason: format!("invalid direction: {}", d),
                    });
                }
            }
            if !seen_codes.insert(entry.code.clone()) {
                return Err(CatalogParseError::DuplicateCode(entry.code.clone()));
            }
            if !seen_repos.insert(entry.repo.clone()) {
                return Err(CatalogParseError::DuplicateRepo(entry.repo.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_catalog() {
        let yaml = r#"
schema_version: 1
languages:
  - code: en
    display_name: English
    repo: pankosmia/en
"#;
        let cat = CatalogFile::parse_yaml(yaml).unwrap();
        assert_eq!(cat.languages.len(), 1);
        assert_eq!(cat.languages[0].code, "en");
    }

    #[test]
    fn rejects_unsupported_schema_version() {
        let yaml = "schema_version: 99\nlanguages: []";
        assert!(matches!(
            CatalogFile::parse_yaml(yaml),
            Err(CatalogParseError::UnsupportedSchema(99))
        ));
    }

    #[test]
    fn rejects_duplicate_code() {
        let yaml = r#"
schema_version: 1
languages:
  - { code: en, display_name: English, repo: a/en }
  - { code: en, display_name: English, repo: b/en }
"#;
        assert!(matches!(
            CatalogFile::parse_yaml(yaml),
            Err(CatalogParseError::DuplicateCode(_))
        ));
    }

    #[test]
    fn rejects_duplicate_repo() {
        let yaml = r#"
schema_version: 1
languages:
  - { code: en, display_name: English, repo: a/x }
  - { code: fr, display_name: French,  repo: a/x }
"#;
        assert!(matches!(
            CatalogFile::parse_yaml(yaml),
            Err(CatalogParseError::DuplicateRepo(_))
        ));
    }

    #[test]
    fn rejects_bad_repo_shape() {
        let yaml = r#"
schema_version: 1
languages:
  - { code: en, display_name: English, repo: just-a-name }
"#;
        assert!(matches!(
            CatalogFile::parse_yaml(yaml),
            Err(CatalogParseError::BadEntry { .. })
        ));
    }

    #[test]
    fn rejects_bad_direction() {
        let yaml = r#"
schema_version: 1
languages:
  - { code: en, display_name: English, repo: a/x, direction: sideways }
"#;
        assert!(matches!(
            CatalogFile::parse_yaml(yaml),
            Err(CatalogParseError::BadEntry { .. })
        ));
    }
}
