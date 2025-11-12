use std::sync::Arc;
use crate::index::index_reader::IndexReader;
use crate::index::lazy_index_reader::LazyIndexReader;
use crate::index::inverted::Term;
use crate::index::posting::Posting;
use crate::storage::layout::StorageLayout;
use crate::storage::segment::SegmentId;
use crate::core::error::Result;

/// Loading strategy for hybrid index reader
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingStrategy {
    /// Load entire index into memory (fast, high memory)
    Eager,
    /// Load on-demand with LRU cache (low memory, slight latency)
    Lazy,
    /// Automatically choose based on index size
    Adaptive,
}

/// Hybrid index reader that switches between eager and lazy loading
pub enum HybridIndexReader {
    Eager(IndexReader),
    Lazy(LazyIndexReader),
}

impl HybridIndexReader {
    /// Open index with automatic strategy selection
    pub fn open(
        storage: &StorageLayout,
        segment_id: SegmentId,
        strategy: LoadingStrategy,
    ) -> Result<Self> {
        Self::open_with_cache_size(storage, segment_id, strategy, 1000)
    }
    
    /// Open index with custom cache size
    pub fn open_with_cache_size(
        storage: &StorageLayout,
        segment_id: SegmentId,
        strategy: LoadingStrategy,
        cache_size: usize,
    ) -> Result<Self> {
        let index_path = storage.index_path(&segment_id);
        
        // Determine actual strategy
        let actual_strategy = match strategy {
            LoadingStrategy::Adaptive => {
                // Auto-select based on file size
                if let Ok(metadata) = std::fs::metadata(&index_path) {
                    let size_mb = metadata.len() / (1024 * 1024);
                    if size_mb < 50 {
                        LoadingStrategy::Eager  // < 50MB → load all
                    } else {
                        LoadingStrategy::Lazy   // >= 50MB → lazy load
                    }
                } else {
                    LoadingStrategy::Eager  // File doesn't exist → eager (empty)
                }
            },
            other => other,
        };
        
        // Create reader based on strategy
        match actual_strategy {
            LoadingStrategy::Eager => {
                let reader = IndexReader::open(storage, segment_id)?;
                Ok(HybridIndexReader::Eager(reader))
            },
            LoadingStrategy::Lazy => {
                let reader = LazyIndexReader::open(storage, segment_id, cache_size)?;
                Ok(HybridIndexReader::Lazy(reader))
            },
            LoadingStrategy::Adaptive => unreachable!(), // Already resolved above
        }
    }
    
    /// Get postings for a term
    pub fn get_postings(&self, term: &Term) -> Result<Option<Arc<Vec<Posting>>>> {
        match self {
            HybridIndexReader::Eager(reader) => {
                Ok(reader.get_postings(term).map(|p| Arc::new(p.clone())))
            },
            HybridIndexReader::Lazy(reader) => {
                reader.get_postings(term)
            },
        }
    }
    
    /// Check if term exists
    pub fn contains_term(&self, term: &Term) -> bool {
        match self {
            HybridIndexReader::Eager(reader) => reader.contains_term(term),
            HybridIndexReader::Lazy(reader) => reader.contains_term(term),
        }
    }
    
    /// Get all terms
    pub fn terms(&self) -> Vec<Term> {
        match self {
            HybridIndexReader::Eager(reader) => {
                reader.terms().into_iter().cloned().collect()
            },
            HybridIndexReader::Lazy(reader) => {
                reader.terms()
            },
        }
    }
    
    /// Get segment ID
    pub fn segment_id(&self) -> SegmentId {
        match self {
            HybridIndexReader::Eager(reader) => reader.segment_id,
            HybridIndexReader::Lazy(reader) => reader.segment_id,
        }
    }
    
    /// Get loading strategy used
    pub fn strategy(&self) -> LoadingStrategy {
        match self {
            HybridIndexReader::Eager(_) => LoadingStrategy::Eager,
            HybridIndexReader::Lazy(_) => LoadingStrategy::Lazy,
        }
    }
    
    /// Get cache statistics (only for lazy mode)
    pub fn cache_stats(&self) -> Option<crate::index::lazy_index_reader::CacheStats> {
        match self {
            HybridIndexReader::Lazy(reader) => Some(reader.cache_stats()),
            HybridIndexReader::Eager(_) => None,
        }
    }
    
    /// Get index statistics
    pub fn stats(&self) -> HybridIndexStats {
        match self {
            HybridIndexReader::Eager(reader) => {
                let stats = reader.stats();
                HybridIndexStats {
                    unique_terms: stats.unique_terms,
                    total_postings: stats.total_postings,
                    strategy: LoadingStrategy::Eager,
                    cache_hit_rate: None,
                }
            },
            HybridIndexReader::Lazy(reader) => {
                let stats = reader.stats();
                let cache_stats = reader.cache_stats();
                HybridIndexStats {
                    unique_terms: stats.unique_terms,
                    total_postings: 0, // Not available in lazy mode without loading all
                    strategy: LoadingStrategy::Lazy,
                    cache_hit_rate: Some(cache_stats.hit_rate),
                }
            },
        }
    }
}

pub struct HybridIndexStats {
    pub unique_terms: usize,
    pub total_postings: usize,
    pub strategy: LoadingStrategy,
    pub cache_hit_rate: Option<f64>,
}
