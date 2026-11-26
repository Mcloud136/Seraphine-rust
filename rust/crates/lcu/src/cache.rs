use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

const MAX_CACHE_ENTRIES: usize = 64;
const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
pub struct CacheCounters {
    hits: AtomicU64,
    coalesced: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CacheMetrics {
    pub hits: u64,
    pub coalesced: u64,
}

impl CacheCounters {
    fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_coalesced(&self) {
        self.coalesced.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> CacheMetrics {
        CacheMetrics {
            hits: self.hits.load(Ordering::Relaxed),
            coalesced: self.coalesced.load(Ordering::Relaxed),
        }
    }
}

struct CacheEntry {
    value: Value,
    size: usize,
    expires_at: Instant,
}

pub enum Start {
    Cached(Value),
    Leader,
    Wait(Arc<Notify>),
}

pub struct CacheFlight {
    cache: Arc<JsonCache>,
    key: String,
    completed: bool,
}

impl CacheFlight {
    pub fn new(cache: Arc<JsonCache>, key: String) -> Self {
        Self {
            cache,
            key,
            completed: false,
        }
    }

    pub fn disarm(mut self) {
        self.completed = true;
    }
}

impl Drop for CacheFlight {
    fn drop(&mut self) {
        if !self.completed {
            self.cache.fail(&self.key);
        }
    }
}

#[derive(Default)]
pub struct JsonCache {
    values: Mutex<HashMap<String, CacheEntry>>,
    inflight: Mutex<HashMap<String, Arc<Notify>>>,
    counters: CacheCounters,
}

impl JsonCache {
    pub fn metrics(&self) -> CacheMetrics {
        self.counters.snapshot()
    }

    pub fn ttl_for(path: &str) -> Duration {
        if path.contains("/lol-match-history/v1/games/") {
            Duration::from_secs(24 * 60 * 60)
        } else if path.contains("/matches") || path.contains("/ranked-stats") {
            Duration::from_secs(5)
        } else {
            Duration::from_millis(200)
        }
    }

    pub fn start(&self, key: &str) -> Start {
        let now = Instant::now();
        let mut values = self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = values.get(key) {
            if entry.expires_at > now {
                self.counters.record_hit();
                return Start::Cached(entry.value.clone());
            }
            values.remove(key);
        }
        drop(values);

        let mut inflight = self
            .inflight
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(notify) = inflight.get(key) {
            self.counters.record_coalesced();
            return Start::Wait(Arc::clone(notify));
        }
        inflight.insert(key.to_string(), Arc::new(Notify::new()));
        Start::Leader
    }

    pub fn complete(&self, key: &str, value: Value, size: usize, ttl: Duration) {
        if size <= MAX_CACHE_BYTES {
            let mut values = self
                .values
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            values.insert(
                key.to_string(),
                CacheEntry {
                    value,
                    size,
                    expires_at: Instant::now() + ttl,
                },
            );
            while values.len() > MAX_CACHE_ENTRIES
                || values.values().map(|entry| entry.size).sum::<usize>() > MAX_CACHE_BYTES
            {
                let Some(evict) = values.keys().next().cloned() else {
                    break;
                };
                values.remove(&evict);
            }
        }
        self.finish(key);
    }

    pub fn fail(&self, key: &str) {
        self.finish(key);
    }

    fn finish(&self, key: &str) {
        let notify = {
            let mut inflight = self
                .inflight
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            inflight.remove(key)
        };
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coalesces_inflight_and_caches_completed_value() {
        let cache = JsonCache::default();
        let ttl = Duration::from_secs(1);
        assert!(matches!(cache.start("key"), Start::Leader));
        assert!(matches!(cache.start("key"), Start::Wait(_)));
        cache.complete("key", json!({"ok": true}), 12, ttl);
        assert!(matches!(cache.start("key"), Start::Cached(_)));
        let metrics = cache.metrics();
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.coalesced, 1);
    }

    #[test]
    fn match_details_use_long_ttl() {
        assert!(JsonCache::ttl_for("/lol-match-history/v1/games/123") > Duration::from_secs(60));
        assert!(JsonCache::ttl_for("/lol-gameflow/v1/gameflow-phase") < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn dropped_leader_releases_inflight_waiters() {
        let cache = Arc::new(JsonCache::default());
        assert!(matches!(cache.start("key"), Start::Leader));
        let flight = CacheFlight::new(Arc::clone(&cache), "key".to_string());
        drop(flight);
        assert!(matches!(cache.start("key"), Start::Leader));
        assert_eq!(cache.metrics().coalesced, 0);
    }
}
