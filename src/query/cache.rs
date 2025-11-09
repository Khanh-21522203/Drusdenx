use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use crate::search::results::SearchResults;

/// Query cache for avoiding recomputation
pub struct QueryCache {
    pub cache: Arc<RwLock<LruCache<QueryCacheKey, SearchResults>>>,
    pub size_limit: usize,
    pub hit_count: AtomicUsize,
    pub miss_count: AtomicUsize,
}

/// Optimized cache key using hash instead of String
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct QueryCacheKey {
    pub query_hash: u64,  // Hash of the query string
    pub limit: usize,
    pub offset: usize,
}

impl QueryCacheKey {
    /// Create cache key from query string without allocating
    pub fn new(query_str: &str, limit: usize, offset: usize) -> Self {
        let mut hasher = DefaultHasher::new();
        query_str.hash(&mut hasher);
        QueryCacheKey {
            query_hash: hasher.finish(),
            limit,
            offset,
        }
    }
}

/// Legacy QueryKey for backward compatibility
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct QueryKey {
    pub query: String,
    pub limit: usize,
    pub offset: usize,
}

impl From<QueryKey> for QueryCacheKey {
    fn from(key: QueryKey) -> Self {
        QueryCacheKey::new(&key.query, key.limit, key.offset)
    }
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

    pub fn get(&self, key: &QueryCacheKey) -> Option<SearchResults> {
        // Try read lock first (multiple readers can access concurrently)
        {
            let cache = self.cache.read().unwrap();
            if let Some(results) = cache.peek(key) {
                // peek() doesn't mutate LRU order -> can use read lock
                self.hit_count.fetch_add(1, Ordering::Relaxed);
                return Some(results.clone());
            }
        }
        
        // Cache miss - no need to update anything
        self.miss_count.fetch_add(1, Ordering::Relaxed);
        None
        
        // Note: If need to update LRU order (get() instead of peek()),
        // could use RwLock + interior mutability pattern (more complex).
        // With LRU cache, peek() is acceptable trade-off.
    }
    
    /// Get from cache using string (avoids allocation)
    pub fn get_by_str(&self, query_str: &str, limit: usize, offset: usize) -> Option<SearchResults> {
        let key = QueryCacheKey::new(query_str, limit, offset);
        self.get(&key)
    }

    pub fn put(&self, key: QueryCacheKey, results: SearchResults) {
        let mut cache = self.cache.write().unwrap();
        cache.put(key, results);
    }
    
    /// Put to cache using string (avoids allocation)
    pub fn put_by_str(&self, query_str: &str, limit: usize, offset: usize, results: SearchResults) {
        let key = QueryCacheKey::new(query_str, limit, offset);
        self.put(key, results);
    }
    
    /// Legacy support - accepts old QueryKey and converts to QueryCacheKey
    pub fn put_legacy(&self, key: QueryKey, results: SearchResults) {
        self.put(key.into(), results);
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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