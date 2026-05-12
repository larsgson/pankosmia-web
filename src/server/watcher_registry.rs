//! Shared file-watcher registry.
//!
//! In v0.15.0 each SSE connection spun up its own `notify` watcher.
//! At ~50 users watching the same file, that's 50 separate inotify
//! subscriptions for one file. Linux's `fs.inotify.max_user_watches`
//! defaults to 8,192 (and is a per-user kernel limit), so the
//! arithmetic fails fast at scale.
//!
//! This registry collapses the watcher count to one-per-(directory,
//! filename) regardless of subscriber count. Subscribers get a
//! `tokio::sync::broadcast::Receiver`. The watcher is torn down
//! when the last subscriber drops.
//!
//! Memory cost: ~1 KB per active subscription key. 100 active
//! ingredients being watched = ~100 KB. Trivial vs. the inotify-
//! count saving.
//!
//! Thread-safety: outer `parking_lot::RwLock` for cheap concurrent
//! lookups; subscription state is `Arc`-cloned out of the map and
//! the inner channel is `tokio::sync::broadcast` (lock-free for
//! senders).

use notify::{Event as NotifyEvent, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use tokio::sync::broadcast;

/// Key identifying a watched file: `(parent_dir, filename)`. The
/// watcher itself is on the parent directory (so it survives the
/// brief file-missing window during atomic-rename saves) and we
/// filter events by the filename component.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WatchKey {
    pub dir: PathBuf,
    pub file: std::ffi::OsString,
}

impl WatchKey {
    pub fn for_file(target: &std::path::Path) -> Option<Self> {
        let parent = target.parent()?.to_path_buf();
        let file = target.file_name()?.to_os_string();
        Some(WatchKey { dir: parent, file })
    }
}

/// Lightweight event payload broadcast to subscribers. Carries no
/// path data — receivers already know what they're watching.
#[derive(Clone, Copy, Debug)]
pub struct ChangeNotice;

struct Subscription {
    sender: broadcast::Sender<ChangeNotice>,
    /// Holding the watcher keeps the inotify subscription alive.
    /// Dropping it tears the OS-level watch down.
    _watcher: RecommendedWatcher,
}

pub struct WatcherRegistry {
    inner: RwLock<HashMap<WatchKey, Weak<Subscription>>>,
}

impl WatcherRegistry {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Subscribe to changes on a target file. Returns:
    ///   - `Ok((handle, receiver))` — keep `handle` alive to keep
    ///     the subscription open. When the last `handle` for a key
    ///     is dropped, the watcher is torn down.
    ///   - `Err(...)` — `notify` couldn't start the watcher (e.g.
    ///     directory doesn't exist).
    pub fn subscribe(
        &self,
        target: &std::path::Path,
    ) -> Result<(SubscriptionHandle, broadcast::Receiver<ChangeNotice>), notify::Error> {
        let key = WatchKey::for_file(target)
            .ok_or_else(|| notify::Error::generic("path has no parent or file name"))?;

        // Fast path: existing subscription, upgrade the weak ref.
        if let Some(existing) = self.lookup(&key) {
            let rx = existing.sender.subscribe();
            return Ok((SubscriptionHandle { _arc: existing }, rx));
        }

        // Slow path: create. Re-check under write lock to avoid a
        // race with a concurrent subscriber.
        let mut w = self.inner.write();
        if let Some(existing) = w.get(&key).and_then(|wk| wk.upgrade()) {
            let rx = existing.sender.subscribe();
            return Ok((SubscriptionHandle { _arc: existing }, rx));
        }

        let (tx, rx) = broadcast::channel::<ChangeNotice>(64);
        let tx_for_watcher = tx.clone();
        let target_filename = key.file.clone();
        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: notify::Result<NotifyEvent>| {
                let ev = match res {
                    Ok(ev) => ev,
                    Err(_) => return,
                };
                if !is_meaningful(&ev.kind) {
                    return;
                }
                let touches_target = ev
                    .paths
                    .iter()
                    .any(|p| p.file_name() == Some(target_filename.as_os_str()));
                if !touches_target {
                    return;
                }
                let _ = tx_for_watcher.send(ChangeNotice);
            })?;
        watcher.watch(&key.dir, RecursiveMode::NonRecursive)?;

        let sub = Arc::new(Subscription {
            sender: tx,
            _watcher: watcher,
        });
        w.insert(key, Arc::downgrade(&sub));
        Ok((SubscriptionHandle { _arc: sub }, rx))
    }

    fn lookup(&self, key: &WatchKey) -> Option<Arc<Subscription>> {
        self.inner.read().get(key).and_then(|wk| wk.upgrade())
    }

    /// Periodic sweep — drops dead Weak entries. Call from a task
    /// or fairing; it's not strictly required (Weak entries are
    /// harmless) but keeps the map size bounded over time.
    pub fn sweep(&self) {
        let mut w = self.inner.write();
        w.retain(|_, weak| weak.strong_count() > 0);
    }

    /// Number of live subscriptions (debug / metrics).
    pub fn live_count(&self) -> usize {
        self.inner
            .read()
            .values()
            .filter(|w| w.strong_count() > 0)
            .count()
    }
}

impl Default for WatcherRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Opaque RAII handle. Drop to release the subscription.
pub struct SubscriptionHandle {
    _arc: Arc<Subscription>,
}

fn is_meaningful(kind: &EventKind) -> bool {
    // Filter for kinds that signal "the file's content might have
    // changed." Access events (file opened for read) are noise.
    matches!(
        kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    #[tokio::test]
    async fn shared_subscription_for_same_file() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("a.txt");
        std::fs::write(&target, b"a").unwrap();
        let reg = WatcherRegistry::new();

        let (_h1, mut rx1) = reg.subscribe(&target).unwrap();
        let (_h2, mut rx2) = reg.subscribe(&target).unwrap();
        // Same key → one subscription.
        assert_eq!(reg.live_count(), 1);

        std::fs::write(&target, b"b").unwrap();

        // Both receivers see the event.
        let _ = tokio::time::timeout(Duration::from_secs(2), rx1.recv())
            .await
            .expect("rx1 timeout")
            .expect("rx1 recv");
        let _ = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
            .await
            .expect("rx2 timeout")
            .expect("rx2 recv");
    }

    #[tokio::test]
    async fn subscription_dropped_when_handle_drops() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("b.txt");
        std::fs::write(&target, b"a").unwrap();
        let reg = WatcherRegistry::new();

        {
            let (_h, _rx) = reg.subscribe(&target).unwrap();
            assert_eq!(reg.live_count(), 1);
        }
        // Handle dropped → subscription torn down. The Weak entry
        // stays in the map until sweep, but live_count sees it as
        // dead.
        assert_eq!(reg.live_count(), 0);
        reg.sweep();
        assert_eq!(reg.inner.read().len(), 0);
    }

    #[tokio::test]
    async fn different_files_get_distinct_subscriptions() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a.txt");
        let b = tmp.path().join("b.txt");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let reg = WatcherRegistry::new();

        let (_ha, _rxa) = reg.subscribe(&a).unwrap();
        let (_hb, _rxb) = reg.subscribe(&b).unwrap();
        assert_eq!(reg.live_count(), 2);
    }
}
