use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use parking_lot::RwLock as ParkingLotRwLock;
use crate::index::index_reader::IndexReader;
use crate::storage::segment::SegmentId;
use crate::storage::layout::StorageLayout;
use crate::core::error::Result;

/// LRU cache for IndexReader
pub struct IndexCache {
    cache: Arc<ParkingLotRwLock<HashMap<SegmentId, Arc<IndexReader>>>>,
    max_size: usize,
    storage: Arc<StorageLayout>,
}

impl IndexCache {
    pub fn new(storage: Arc<StorageLayout>, max_size: usize) -> Self {
        IndexCache {
            cache: Arc::new(ParkingLotRwLock::new(HashMap::new())),
            max_size,
            storage,
        }
    }
    
    /// Get or load IndexReader from cache
    pub fn get_or_load(&self, segment_id: SegmentId) -> Result<Arc<IndexReader>> {
        // Fast path: check if already cached
        {
            let cache = self.cache.read();
            if let Some(reader) = cache.get(&segment_id) {
                return Ok(reader.clone());
            }
        }
        
        // Slow path: load from disk
        let reader = IndexReader::open(&self.storage, segment_id)?;
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
    pub fn stats(&self) -> CacheStats {
        let cache = self.cache.read();
        CacheStats {
            size: cache.len(),
            max_size: self.max_size,
        }
    }
}

pub struct CacheStats {
    pub size: usize,
    pub max_size: usize,
}

impl Clone for IndexCache {
    fn clone(&self) -> Self {
        IndexCache {
            cache: self.cache.clone(),
            max_size: self.max_size,
            storage: self.storage.clone(),
        }
    }
}
