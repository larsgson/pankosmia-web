use crate::structs::MetadataSummary;
use moka::sync::Cache;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

pub struct GiteaCache {
    pub summaries: Cache<String, Arc<BTreeMap<String, MetadataSummary>>>,
    pub trees: Cache<String, Arc<Vec<String>>>,
    pub raw_files: Cache<String, Arc<(String, Vec<u8>)>>,
}

impl GiteaCache {
    pub fn new() -> Self {
        Self {
            summaries: Cache::builder()
                .time_to_live(Duration::from_secs(600))
                .max_capacity(100)
                .build(),
            trees: Cache::builder()
                .time_to_live(Duration::from_secs(300))
                .max_capacity(500)
                .build(),
            raw_files: Cache::builder()
                .time_to_live(Duration::from_secs(120))
                .max_capacity(2000)
                .build(),
        }
    }
}
