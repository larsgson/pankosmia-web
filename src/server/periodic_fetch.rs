//! Periodic refresh of the local cache for every catalog-registered
//! language.
//!
//! Background task that wakes up every
//! `PANKOSMIA_PERIODIC_FETCH_INTERVAL_SECS` seconds (default 900 =
//! 15 minutes), walks `CatalogRegistry::list()`, and calls
//! `ProjectStore::prefetch_language` on each. Mtime changes from
//! the resulting `git fetch` are picked up by the WatcherRegistry
//! and broadcast as SSE `change` events to any subscribers.
//!
//! Fallback for missed webhook deliveries.
//!
//! Disable by setting `PANKOSMIA_PERIODIC_FETCH_INTERVAL_SECS=0`.

use crate::catalog::{CatalogRegistry, SharedCatalogSync};
use crate::store::SharedProjectStore;
use std::sync::Arc;
use std::time::Duration;

/// Parameters for org-based catalog discovery (when
/// `PANKOSMIA_CATALOG_ORG` is set). Passed to `spawn` so the
/// periodic task can refresh the catalog via the GitHub Search API
/// instead of a git pull.
pub struct OrgDiscoveryParams {
    pub org: String,
    pub installation_id: u64,
}

/// Default cadence: 15 minutes. Matches the latency guidance given
/// to clients in `docs/CLIENT_INTEGRATION.md` §7.
pub const DEFAULT_INTERVAL_SECS: u64 = 15 * 60;

/// Stagger before the first tick so a fleet restart doesn't hammer
/// GitHub all at once.
const STARTUP_JITTER: Duration = Duration::from_secs(30);

/// Read the configured interval from env. `None` means "disabled"
/// (env var explicitly `0`).
pub fn interval_from_env() -> Option<Duration> {
    let secs = std::env::var("PANKOSMIA_PERIODIC_FETCH_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    }
}

/// Spawn the background task. Returns immediately. The task runs
/// for the lifetime of the process; it has no shutdown signal in
/// this version (process exit drops it).
///
/// When `org_params` is `Some`, the catalog is refreshed via GitHub
/// Search API (org discovery) instead of a git pull.
pub fn spawn(
    catalog: Arc<CatalogRegistry>,
    catalog_sync: SharedCatalogSync,
    store: SharedProjectStore,
    interval: Duration,
    org_params: Option<OrgDiscoveryParams>,
) {
    println!("periodic_fetch: running every {}s", interval.as_secs());
    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_JITTER).await;
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;

        let discovery_auth = if org_params.is_some() {
            match crate::auth::GithubAppAuth::from_env() {
                Ok(Some(a)) => Some(a),
                _ => {
                    eprintln!("periodic_fetch: org discovery configured but App auth unavailable");
                    None
                }
            }
        } else {
            None
        };

        loop {
            ticker.tick().await;
            if let (Some(params), Some(auth)) = (&org_params, &discovery_auth) {
                match crate::catalog::discovery::discover_languages(
                    auth,
                    params.installation_id,
                    &params.org,
                    &catalog,
                )
                .await
                {
                    Ok(diff) => {
                        if !diff.added.is_empty() || !diff.removed.is_empty() {
                            println!(
                                "periodic_fetch: discovery refresh: +{} -{}",
                                diff.added.len(),
                                diff.removed.len()
                            );
                        }
                    }
                    Err(e) => eprintln!("periodic_fetch: discovery refresh failed: {}", e),
                }
            } else {
                let catalog_for_sync = catalog.clone();
                let sync_clone = catalog_sync.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if let Err(e) = sync_clone.refresh(&catalog_for_sync) {
                        eprintln!("periodic_fetch: catalog refresh failed: {}", e);
                    }
                })
                .await;
            }
            let languages = catalog.list();
            if languages.is_empty() {
                continue;
            }
            for lang in languages {
                if let Err(e) = store.prefetch_language(lang.code.clone()).await {
                    eprintln!(
                        "periodic_fetch: prefetch_language({}) failed: {}",
                        lang.code.as_str(),
                        e
                    );
                }
            }
        }
    });
}
