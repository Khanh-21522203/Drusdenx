use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use parking_lot::Mutex;
use lru::LruCache;
use std::num::NonZeroUsize;
use crate::compression::compress::CompressedBlock;
use crate::index::inverted::Term;
use crate::index::posting::Posting;
use crate::storage::layout::StorageLayout;
use crate::storage::segment::SegmentId;
use crate::core::error::Result;

/// Lazy loading index reader with LRU cache
pub struct LazyIndexReader {
    pub segment_id: SegmentId,
    term_offsets: HashMap<Term, TermOffset>,  // Term -> file offset
    file: Arc<Mutex<File>>,
    cache: Arc<Mutex<LruCache<Term, Arc<Vec<Posting>>>>>,  // LRU cache for postings
    cache_hits: std::sync::atomic::AtomicU64,
    cache_misses: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
struct TermOffset {
    offset: u64,
    length: u64,
}

impl LazyIndexReader {
    /// Open index file and load only the term dictionary (lightweight)
    pub fn open(storage: &StorageLayout, segment_id: SegmentId, cache_size: usize) -> Result<Self> {
        let index_path = storage.index_path(&segment_id);
        
        // Check if index file exists
        if !index_path.exists() {
            return Ok(LazyIndexReader {
                segment_id,
                term_offsets: HashMap::new(),
                file: Arc::new(Mutex::new(File::open("/dev/null")?)),
                cache: Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(1).unwrap()))),
                cache_hits: std::sync::atomic::AtomicU64::new(0),
                cache_misses: std::sync::atomic::AtomicU64::new(0),
            });
        }
        
        // Read the full index file (we'll optimize this later)
        let mut index_file = File::open(&index_path)?;
        let mut compressed_block_data = Vec::new();
        index_file.read_to_end(&mut compressed_block_data)?;
        
        // Deserialize and decompress
        let compressed_block: CompressedBlock = bincode::deserialize(&compressed_block_data)?;
        let decompressed = CompressedBlock::decompress(&compressed_block)?;
        
        // Deserialize to get full index
        let full_index: HashMap<Term, Vec<Posting>> = bincode::deserialize(&decompressed)?;
        
        // Build term offsets dictionary (for now, we'll store serialized data per term)
        let mut term_offsets = HashMap::new();
        let mut term_data_map = HashMap::new();
        
        for (term, postings) in full_index.into_iter() {
            // Serialize each term's postings
            let serialized = bincode::serialize(&postings)?;
            let offset = 0u64; // Will be used later with proper file format
            let length = serialized.len() as u64;
            
            term_offsets.insert(term.clone(), TermOffset { offset, length });
            term_data_map.insert(term, serialized);
        }
        
        // Re-open file for seeking
        let file = File::open(&index_path)?;
        
        Ok(LazyIndexReader {
            segment_id,
            term_offsets,
            file: Arc::new(Mutex::new(file)),
            cache: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(cache_size).unwrap_or(NonZeroUsize::new(1000).unwrap())
            ))),
            cache_hits: std::sync::atomic::AtomicU64::new(0),
            cache_misses: std::sync::atomic::AtomicU64::new(0),
        })
    }
    
    /// Get postings for a term (with caching)
    pub fn get_postings(&self, term: &Term) -> Result<Option<Arc<Vec<Posting>>>> {
        // Check cache first
        {
            let mut cache = self.cache.lock();
            if let Some(postings) = cache.get(term) {
                self.cache_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(Some(postings.clone()));
            }
        }
        
        // Cache miss - load from "disk" (currently from deserialized data)
        self.cache_misses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        
        if let Some(_term_offset) = self.term_offsets.get(term) {
            // For now, we'll re-read the full index (temporary implementation)
            // TODO: Implement proper offset-based reading
            let postings = self.load_postings_for_term(term)?;
            
            if let Some(postings) = postings {
                let arc_postings = Arc::new(postings);
                
                // Cache for future use
                let mut cache = self.cache.lock();
                cache.put(term.clone(), arc_postings.clone());
                
                return Ok(Some(arc_postings));
            }
        }
        
        Ok(None)
    }
    
    /// Load postings for a specific term from file
    fn load_postings_for_term(&self, term: &Term) -> Result<Option<Vec<Posting>>> {
        // Temporary: re-read full index (will optimize with proper file format later)
        let mut file = self.file.lock();
        file.seek(SeekFrom::Start(0))?;
        
        let mut compressed_block_data = Vec::new();
        file.read_to_end(&mut compressed_block_data)?;
        
        let compressed_block: CompressedBlock = bincode::deserialize(&compressed_block_data)?;
        let decompressed = CompressedBlock::decompress(&compressed_block)?;
        let full_index: HashMap<Term, Vec<Posting>> = bincode::deserialize(&decompressed)?;
        
        Ok(full_index.get(term).cloned())
    }
    
    /// Check if term exists
    pub fn contains_term(&self, term: &Term) -> bool {
        self.term_offsets.contains_key(term)
    }
    
    /// Get all terms (from dictionary only - no loading needed)
    pub fn terms(&self) -> Vec<Term> {
        self.term_offsets.keys().cloned().collect()
    }
    
    /// Get cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        let hits = self.cache_hits.load(std::sync::atomic::Ordering::Relaxed);
        let misses = self.cache_misses.load(std::sync::atomic::Ordering::Relaxed);
        let total = hits + misses;
        let hit_rate = if total > 0 {
            (hits as f64) / (total as f64)
        } else {
            0.0
        };
        
        CacheStats {
            hits,
            misses,
            hit_rate,
            size: self.cache.lock().len(),
        }
    }
    
    /// Get index statistics
    pub fn stats(&self) -> IndexStats {
        IndexStats {
            unique_terms: self.term_offsets.len(),
        }
    }
}

pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: f64,
    pub size: usize,
}

pub struct IndexStats {
    pub unique_terms: usize,
}
