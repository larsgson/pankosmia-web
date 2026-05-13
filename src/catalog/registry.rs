//! In-memory `CatalogRegistry` — the runtime view of registered
//! languages.
//!
//! Built from the parsed `CatalogFile`. Indexed by `LanguageCode`
//! for O(1) lookup. Refreshed by reloading the `languages.yaml`
//! after a `git pull` of the catalog repo.
//!
//! This module deliberately knows nothing about git2 or webhooks —
//! it's a pure data structure. The git plumbing lives in
//! `crate::store::github::catalog_clone` (added in G2's
//! integration glue).

use crate::catalog::schema::{CatalogEntry, CatalogFile, CatalogParseError};
use crate::identity::LanguageCode;
use parking_lot::RwLock;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct RegisteredLanguage {
    pub code: LanguageCode,
    pub display_name: String,
    pub repo: String, // "owner/name"
    pub script: Option<String>,
    pub direction: Option<String>,
    /// Per-language GitHub App installation override.
    pub installation_id: Option<u64>,
}

impl RegisteredLanguage {
    fn from_entry(e: &CatalogEntry) -> Result<Self, CatalogParseError> {
        Ok(RegisteredLanguage {
            code: LanguageCode::parse(&e.code).map_err(|e2| CatalogParseError::BadEntry {
                index: usize::MAX,
                reason: e2.to_string(),
            })?,
            display_name: e.display_name.clone(),
            repo: e.repo.clone(),
            script: e.script.clone(),
            direction: e.direction.clone(),
            installation_id: e.installation_id,
        })
    }

    pub fn upstream_clone_url(&self) -> String {
        format!("https://github.com/{}.git", self.repo)
    }
}

pub struct CatalogRegistry {
    by_code: RwLock<HashMap<LanguageCode, RegisteredLanguage>>,
}

impl CatalogRegistry {
    pub fn empty() -> Self {
        Self {
            by_code: RwLock::new(HashMap::new()),
        }
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, CatalogParseError> {
        let cat = CatalogFile::parse_yaml(yaml)?;
        let map = cat
            .languages
            .iter()
            .map(|e| {
                let r = RegisteredLanguage::from_entry(e)?;
                Ok((r.code.clone(), r))
            })
            .collect::<Result<HashMap<_, _>, CatalogParseError>>()?;
        Ok(Self {
            by_code: RwLock::new(map),
        })
    }

    /// Atomically replace the registry contents with the parsed
    /// contents of a new `languages.yaml`. Returns the diff
    /// (added, removed) for the caller to act on (e.g. tear down
    /// caches for removed languages).
    pub fn reload_from_yaml(&self, yaml: &str) -> Result<RegistryDiff, CatalogParseError> {
        let cat = CatalogFile::parse_yaml(yaml)?;
        let mut new_map = HashMap::new();
        for e in &cat.languages {
            let r = RegisteredLanguage::from_entry(e)?;
            new_map.insert(r.code.clone(), r);
        }
        let mut w = self.by_code.write();
        let old = std::mem::replace(&mut *w, new_map);
        let new_codes: std::collections::HashSet<_> = w.keys().cloned().collect();
        let old_codes: std::collections::HashSet<_> = old.keys().cloned().collect();
        let added: Vec<LanguageCode> = new_codes.difference(&old_codes).cloned().collect();
        let removed: Vec<LanguageCode> = old_codes.difference(&new_codes).cloned().collect();
        Ok(RegistryDiff { added, removed })
    }

    pub fn reload_from_entries(&self, entries: Vec<RegisteredLanguage>) -> RegistryDiff {
        let mut new_map = HashMap::new();
        for r in entries {
            new_map.insert(r.code.clone(), r);
        }
        let mut w = self.by_code.write();
        let old = std::mem::replace(&mut *w, new_map);
        let new_codes: std::collections::HashSet<_> = w.keys().cloned().collect();
        let old_codes: std::collections::HashSet<_> = old.keys().cloned().collect();
        let added: Vec<LanguageCode> = new_codes.difference(&old_codes).cloned().collect();
        let removed: Vec<LanguageCode> = old_codes.difference(&new_codes).cloned().collect();
        RegistryDiff { added, removed }
    }

    pub fn get(&self, code: &LanguageCode) -> Option<RegisteredLanguage> {
        self.by_code.read().get(code).cloned()
    }

    pub fn contains(&self, code: &LanguageCode) -> bool {
        self.by_code.read().contains_key(code)
    }

    pub fn list(&self) -> Vec<RegisteredLanguage> {
        self.by_code.read().values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.by_code.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_code.read().is_empty()
    }
}

pub struct RegistryDiff {
    pub added: Vec<LanguageCode>,
    pub removed: Vec<LanguageCode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_lists() {
        let yaml = r#"
schema_version: 1
languages:
  - { code: en, display_name: English, repo: a/en }
  - { code: fr, display_name: French,  repo: b/fr }
"#;
        let reg = CatalogRegistry::from_yaml(yaml).unwrap();
        assert_eq!(reg.len(), 2);
        assert!(reg.contains(&LanguageCode::parse("en").unwrap()));
        assert!(!reg.contains(&LanguageCode::parse("zz").unwrap()));
    }

    #[test]
    fn reload_diff_reports_changes() {
        let reg = CatalogRegistry::from_yaml(
            r#"
schema_version: 1
languages:
  - { code: en, display_name: English, repo: a/en }
  - { code: fr, display_name: French,  repo: b/fr }
"#,
        )
        .unwrap();
        let diff = reg
            .reload_from_yaml(
                r#"
schema_version: 1
languages:
  - { code: en, display_name: English, repo: a/en }
  - { code: ar, display_name: Arabic,  repo: c/ar }
"#,
            )
            .unwrap();
        assert_eq!(diff.added, vec![LanguageCode::parse("ar").unwrap()]);
        assert_eq!(diff.removed, vec![LanguageCode::parse("fr").unwrap()]);
    }
}
