use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::search::results::SearchResults;

/// Query cache for avoiding recomputation
pub struct QueryCache {
    pub cache: Arc<RwLock<LruCache<QueryKey, SearchResults>>>,
    pub size_limit: usize,
    pub hit_count: AtomicUsize,
    pub miss_count: AtomicUsize,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct QueryKey {
    pub query: String,
    pub limit: usize,
    pub offset: usize,
}

impl QueryCache {
    pub fn new(size_limit: usize) -> Self {
        let cap = NonZeroUsize::new(size_limit).unwrap();
        QueryCache {
            cache: Arc::new(RwLock::new(LruCache::new(cap))),
            size_limit,
            hit_count: AtomicUsize::new(0),
            miss_count: AtomicUsize::new(0),
        }
    }

    pub fn get(&self, key: &QueryKey) -> Option<SearchResults> {
        let mut cache = self.cache.write().unwrap();
        if let Some(results) = cache.get(key) {
            self.hit_count.fetch_add(1, Ordering::Relaxed);
            Some(results.clone())
        } else {
            self.miss_count.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    pub fn put(&self, key: QueryKey, results: SearchResults) {
        let mut cache = self.cache.write().unwrap();
        cache.put(key, results);
    }

    pub fn clear(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hit_count: self.hit_count.load(Ordering::Relaxed),
            miss_count: self.miss_count.load(Ordering::Relaxed),
            size: self.cache.read().unwrap().len(),
            capacity: self.size_limit,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub hit_count: usize,
    pub miss_count: usize,
    pub size: usize,
    pub capacity: usize,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hit_count + self.miss_count;
        if total == 0 {
            0.0
        } else {
            self.hit_count as f64 / total as f64
        }
    }
}