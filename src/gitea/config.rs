use std::collections::HashSet;

pub struct CuratedOrgs {
    entries: HashSet<String>,
}

impl CuratedOrgs {
    pub fn from_env() -> Self {
        let mut entries = HashSet::new();
        if let Ok(val) = std::env::var("PANKOSMIA_CURATED_ORGS") {
            for entry in val.split(',') {
                let trimmed = entry.trim().trim_end_matches('/');
                if !trimmed.is_empty() {
                    let parts: Vec<&str> = trimmed.split('/').collect();
                    if parts.len() == 2 {
                        entries.insert(trimmed.to_string());
                    } else {
                        eprintln!(
                            "WARN: ignoring malformed PANKOSMIA_CURATED_ORGS entry: {:?}",
                            trimmed
                        );
                    }
                }
            }
        }
        if entries.is_empty() {
            println!("curated_orgs: none configured (PANKOSMIA_CURATED_ORGS unset)");
        } else {
            println!("curated_orgs: {}", entries.iter().cloned().collect::<Vec<_>>().join(", "));
        }
        Self { entries }
    }

    pub fn is_curated(&self, server_org: &str) -> bool {
        self.entries.contains(server_org)
    }

    pub fn iter_orgs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().filter_map(|e| e.split_once('/'))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
