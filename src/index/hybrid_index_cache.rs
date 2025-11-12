use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use crate::index::hybrid_index_reader::{HybridIndexReader, LoadingStrategy};
use crate::storage::segment::SegmentId;
use crate::storage::layout::StorageLayout;
use crate::core::error::Result;

/// Cache for HybridIndexReader with configurable loading strategy
pub struct HybridIndexCache {
    cache: Arc<RwLock<HashMap<SegmentId, Arc<HybridIndexReader>>>>,
    max_size: usize,
    storage: Arc<StorageLayout>,
    default_strategy: LoadingStrategy,
    lazy_cache_size: usize,
}

impl HybridIndexCache {
    pub fn new(
        storage: Arc<StorageLayout>,
        max_size: usize,
        default_strategy: LoadingStrategy,
        lazy_cache_size: usize,
    ) -> Self {
        HybridIndexCache {
            cache: Arc::new(RwLock::new(HashMap::new())),
            max_size,
            storage,
            default_strategy,
            lazy_cache_size,
        }
    }
    
    /// Create with adaptive strategy (recommended)
    pub fn new_adaptive(storage: Arc<StorageLayout>, max_size: usize) -> Self {
        Self::new(storage, max_size, LoadingStrategy::Adaptive, 1000)
    }
    
    /// Get or load HybridIndexReader from cache
    pub fn get_or_load(&self, segment_id: SegmentId) -> Result<Arc<HybridIndexReader>> {
        // Fast path: check if already cached
        {
            let cache = self.cache.read();
            if let Some(reader) = cache.get(&segment_id) {
                return Ok(reader.clone());
            }
        }
        
        // Slow path: load from disk with configured strategy
        let reader = HybridIndexReader::open_with_cache_size(
            &self.storage,
            segment_id,
            self.default_strategy,
            self.lazy_cache_size,
        )?;
        let reader = Arc::new(reader);
        
        // Insert into cache
        {
            let mut cache = self.cache.write();
            
            // Check size and evict if needed (simple FIFO)
            if cache.len() >= self.max_size {
                // Remove oldest entry
                if let Some(key) = cache.keys().next().cloned() {
                    cache.remove(&key);
                }
            }
            
            cache.insert(segment_id, reader.clone());
        }
        
        Ok(reader)
    }
    
    /// Get or load with custom strategy
    pub fn get_or_load_with_strategy(
        &self,
        segment_id: SegmentId,
        strategy: LoadingStrategy,
    ) -> Result<Arc<HybridIndexReader>> {
        // Check cache first
        {
            let cache = self.cache.read();
            if let Some(reader) = cache.get(&segment_id) {
                return Ok(reader.clone());
            }
        }
        
        // Load with custom strategy
        let reader = HybridIndexReader::open_with_cache_size(
            &self.storage,
            segment_id,
            strategy,
            self.lazy_cache_size,
        )?;
        let reader = Arc::new(reader);
        
        // Cache it
        let mut cache = self.cache.write();
        if cache.len() >= self.max_size {
            if let Some(key) = cache.keys().next().cloned() {
                cache.remove(&key);
            }
        }
        cache.insert(segment_id, reader.clone());
        
        Ok(reader)
    }
    
    /// Invalidate cache entry
    pub fn invalidate(&self, segment_id: &SegmentId) {
        let mut cache = self.cache.write();
        cache.remove(segment_id);
    }
    
    /// Clear entire cache
    pub fn clear(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }
    
    /// Get cache statistics
    pub fn stats(&self) -> HybridCacheStats {
        let cache = self.cache.read();
        
        let mut eager_count = 0;
        let mut lazy_count = 0;
        let mut total_cache_hit_rate = 0.0;
        let mut lazy_readers_count = 0;
        
        for reader in cache.values() {
            match reader.strategy() {
                LoadingStrategy::Eager => eager_count += 1,
                LoadingStrategy::Lazy => {
                    lazy_count += 1;
                    if let Some(cache_stats) = reader.cache_stats() {
                        total_cache_hit_rate += cache_stats.hit_rate;
                        lazy_readers_count += 1;
                    }
                },
                LoadingStrategy::Adaptive => {}, // Shouldn't happen after opening
            }
        }
        
        let avg_cache_hit_rate = if lazy_readers_count > 0 {
            Some(total_cache_hit_rate / lazy_readers_count as f64)
        } else {
            None
        };
        
        HybridCacheStats {
            total_segments: cache.len(),
            eager_segments: eager_count,
            lazy_segments: lazy_count,
            max_size: self.max_size,
            default_strategy: self.default_strategy,
            avg_lazy_cache_hit_rate: avg_cache_hit_rate,
        }
    }
}

pub struct HybridCacheStats {
    pub total_segments: usize,
    pub eager_segments: usize,
    pub lazy_segments: usize,
    pub max_size: usize,
    pub default_strategy: LoadingStrategy,
    pub avg_lazy_cache_hit_rate: Option<f64>,
}

impl Clone for HybridIndexCache {
    fn clone(&self) -> Self {
        HybridIndexCache {
            cache: self.cache.clone(),
            max_size: self.max_size,
            storage: self.storage.clone(),
            default_strategy: self.default_strategy,
            lazy_cache_size: self.lazy_cache_size,
        }
    }
}
