use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const NEGATIVE_TTL: Duration = Duration::from_secs(30);
const MAX_ENTRIES: usize = 512;

#[derive(Clone, Copy, Debug, Default)]
pub struct AssetCacheMetrics {
    pub inflight: usize,
    pub negative_entries: usize,
    pub coalesced: u64,
    pub negative_hits: u64,
    pub failures: u64,
}

pub enum AssetStart {
    Leader,
    Wait(watch::Receiver<()>),
    Deferred,
}

pub struct AssetCache {
    inflight: Mutex<HashMap<String, watch::Sender<()>>>,
    failures: Mutex<HashMap<String, Instant>>,
    negative_ttl: Duration,
    coalesced: AtomicU64,
    negative_hits: AtomicU64,
    failures_count: AtomicU64,
}

impl Default for AssetCache {
    fn default() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
            negative_ttl: NEGATIVE_TTL,
            coalesced: AtomicU64::new(0),
            negative_hits: AtomicU64::new(0),
            failures_count: AtomicU64::new(0),
        }
    }
}

impl AssetCache {
    pub fn start(&self, key: &str) -> AssetStart {
        let now = Instant::now();
        {
            let mut failures = self
                .failures
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            failures.retain(|_, expires_at| *expires_at > now);
            if failures.contains_key(key) {
                self.negative_hits.fetch_add(1, Ordering::Relaxed);
                return AssetStart::Deferred;
            }
        }

        let mut inflight = self
            .inflight
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(sender) = inflight.get(key) {
            self.coalesced.fetch_add(1, Ordering::Relaxed);
            return AssetStart::Wait(sender.subscribe());
        }
        if inflight.len() >= MAX_ENTRIES {
            return AssetStart::Deferred;
        }
        let (sender, _) = watch::channel(());
        inflight.insert(key.to_string(), sender);
        AssetStart::Leader
    }

    pub fn complete(&self, key: &str) {
        self.finish(key);
    }

    pub fn clear_failures(&self) {
        self.failures
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }

    pub fn fail(&self, key: &str) {
        let now = Instant::now();
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        failures.retain(|_, expires_at| *expires_at > now);
        failures.insert(key.to_string(), now + self.negative_ttl);
        while failures.len() > MAX_ENTRIES {
            let Some(oldest) = failures
                .iter()
                .min_by_key(|(_, expires_at)| *expires_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            failures.remove(&oldest);
        }
        self.failures_count.fetch_add(1, Ordering::Relaxed);
        drop(failures);
        self.finish(key);
    }

    fn finish(&self, key: &str) {
        let sender = {
            let mut inflight = self
                .inflight
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            inflight.remove(key)
        };
        if let Some(sender) = sender {
            sender.send_replace(());
        }
    }

    pub fn metrics(&self) -> AssetCacheMetrics {
        let inflight = self
            .inflight
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len();
        let now = Instant::now();
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        failures.retain(|_, expires_at| *expires_at > now);
        AssetCacheMetrics {
            inflight,
            negative_entries: failures.len(),
            coalesced: self.coalesced.load(Ordering::Relaxed),
            negative_hits: self.negative_hits.load(Ordering::Relaxed),
            failures: self.failures_count.load(Ordering::Relaxed),
        }
    }
}

pub struct AssetFlight {
    cache: Arc<AssetCache>,
    key: String,
    completed: bool,
}

impl AssetFlight {
    pub fn new(cache: Arc<AssetCache>, key: String) -> Self {
        Self {
            cache,
            key,
            completed: false,
        }
    }

    pub fn complete(mut self) {
        self.cache.complete(&self.key);
        self.completed = true;
    }
}

impl Drop for AssetFlight {
    fn drop(&mut self) {
        if !self.completed {
            self.cache.fail(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn coalesces_waiters_and_wakes_them_on_completion() {
        let cache = AssetCache::default();
        assert!(matches!(cache.start("asset"), AssetStart::Leader));
        let AssetStart::Wait(mut waiter) = cache.start("asset") else {
            panic!("expected waiter");
        };
        cache.complete("asset");
        waiter.changed().await.unwrap();
        assert_eq!(cache.metrics().coalesced, 1);
        assert_eq!(cache.metrics().inflight, 0);
    }

    #[test]
    fn failures_are_negative_cached() {
        let cache = AssetCache {
            negative_ttl: Duration::from_millis(20),
            ..AssetCache::default()
        };
        assert!(matches!(cache.start("asset"), AssetStart::Leader));
        cache.fail("asset");
        assert!(matches!(cache.start("asset"), AssetStart::Deferred));
        std::thread::sleep(Duration::from_millis(40));
        assert!(matches!(cache.start("asset"), AssetStart::Leader));
    }

    #[test]
    fn connection_changes_clear_negative_cache() {
        let cache = AssetCache::default();
        assert!(matches!(cache.start("asset"), AssetStart::Leader));
        cache.fail("asset");
        assert!(matches!(cache.start("asset"), AssetStart::Deferred));
        cache.clear_failures();
        assert!(matches!(cache.start("asset"), AssetStart::Leader));
    }

    #[test]
    fn dropped_flight_releases_waiters_and_negative_caches() {
        let cache = Arc::new(AssetCache::default());
        assert!(matches!(cache.start("asset"), AssetStart::Leader));
        let flight = AssetFlight::new(Arc::clone(&cache), "asset".to_string());
        drop(flight);
        assert_eq!(cache.metrics().failures, 1);
        assert!(matches!(cache.start("asset"), AssetStart::Deferred));
    }
}
