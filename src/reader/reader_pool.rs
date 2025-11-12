use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use roaring::RoaringBitmap;
use crate::mvcc::controller::{MVCCController, Snapshot};
use crate::storage::segment_reader::SegmentReader;
use crate::core::error::Result;
use crate::index::inverted::InvertedIndex;
use crate::query::ast::Query;
use crate::query::matcher::{DocumentMatcher, SegmentSearch};
use crate::search::results::{SearchResults, ScoredDocument};
use crate::storage::layout::StorageLayout;

/// Pool of index readers with caching to prevent memory leak
pub struct ReaderPool {
    pub readers: Arc<RwLock<Vec<Arc<IndexReader>>>>,
    pub mvcc: Arc<MVCCController>,
    pub max_readers: usize,
    pub storage: Arc<StorageLayout>,
    pub index: Arc<InvertedIndex>,
    /// Cache readers by snapshot version to reuse them
    reader_cache: Arc<RwLock<HashMap<u64, Arc<IndexReader>>>>,
    /// Track open segment readers for proper cleanup
    segment_reader_cache: Arc<RwLock<HashMap<(u64, usize), Arc<RwLock<SegmentReader>>>>>,
}

/// Index reader with snapshot
pub struct IndexReader {
    pub snapshot: Arc<Snapshot>,
    pub segments: Vec<Arc<RwLock<SegmentReader>>>,
    pub deleted_docs: Arc<RoaringBitmap>,
    pub index: Arc<InvertedIndex>,
}

impl ReaderPool {
    pub fn new(
        mvcc: Arc<MVCCController>,
        storage: Arc<StorageLayout>,
        index: Arc<InvertedIndex>,
        max_readers: usize
    ) -> Self {
        ReaderPool {
            readers: Arc::new(RwLock::new(Vec::new())),
            mvcc,
            max_readers,
            storage,
            index,
            reader_cache: Arc::new(RwLock::new(HashMap::new())),
            segment_reader_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    pub fn get_reader(&self) -> Result<Arc<IndexReader>> {
        let snapshot = self.mvcc.current_snapshot();
        let version = snapshot.version;
        
        // Check if we have a cached reader for this snapshot version
        {
            let cache = self.reader_cache.read();
            if let Some(cached_reader) = cache.get(&version) {
                return Ok(cached_reader.clone());
            }
        }
        
        // Create new reader if not cached
        let reader = self.create_reader_for_snapshot(snapshot)?;
        
        // Cache the reader for future use
        {
            let mut cache = self.reader_cache.write();
            cache.insert(version, reader.clone());
            
            // Clean up old cached readers if we exceed max_readers
            if cache.len() > self.max_readers {
                self.cleanup_old_readers(&mut cache);
            }
        }
        
        Ok(reader)
    }
    
    /// Create a new IndexReader for the given snapshot
    fn create_reader_for_snapshot(&self, snapshot: Arc<Snapshot>) -> Result<Arc<IndexReader>> {
        let version = snapshot.version;
        let deleted_docs = snapshot.deleted_docs.clone();
        
        // Create or reuse segment readers
        let mut segment_readers = Vec::new();
        for (idx, segment) in snapshot.segments.iter().enumerate() {
            let cache_key = (version, idx);
            
            // Check segment reader cache
            let cached_segment = {
                let cache = self.segment_reader_cache.read();
                cache.get(&cache_key).cloned()
            };
            
            let segment_reader = if let Some(cached) = cached_segment {
                cached
            } else {
                // Create new segment reader, skip if it fails (e.g., empty segment)
                match SegmentReader::open(&self.storage, segment.id) {
                    Ok(reader) => {
                        let reader_arc = Arc::new(RwLock::new(reader));
                        
                        // Cache it
                        let mut cache = self.segment_reader_cache.write();
                        cache.insert(cache_key, reader_arc.clone());
                        
                        reader_arc
                    }
                    Err(_e) => {
                        // Skip segments that can't be opened (e.g., empty segments)
                        continue;
                    }
                }
            };
            
            segment_readers.push(segment_reader);
        }
        
        Ok(Arc::new(IndexReader {
            snapshot,
            segments: segment_readers,
            deleted_docs,
            index: self.index.clone(),
        }))
    }
    
    /// Clean up old readers when cache is full
    fn cleanup_old_readers(&self, cache: &mut HashMap<u64, Arc<IndexReader>>) {
        // Keep only the most recent readers
        let mut versions: Vec<u64> = cache.keys().cloned().collect();
        versions.sort();
        
        // Remove oldest readers, keep max_readers/2 most recent
        let keep_count = self.max_readers / 2;
        if versions.len() > keep_count {
            let to_remove = versions.len() - keep_count;
            for version in versions.iter().take(to_remove) {
                cache.remove(version);
                // Also remove associated segment readers
                self.cleanup_segment_readers(*version);
            }
        }
    }
    
    /// Clean up segment readers for a specific version
    fn cleanup_segment_readers(&self, version: u64) {
        let mut cache = self.segment_reader_cache.write();
        cache.retain(|&(v, _), _| v != version);
    }
}

impl IndexReader {
    pub fn search(&self, query: &Query) -> Result<SearchResults> {
        self.search_with_limit(query, usize::MAX) // No limit by default
    }
    
    pub fn search_with_limit(&self, query: &Query, limit: usize) -> Result<SearchResults> {
        let matcher = DocumentMatcher::new(self.index.clone());
        let mut all_results = Vec::new();
        
        // Early termination optimization: if we have enough high-scoring results,
        // we can stop searching segments early (especially useful for sorted segments)
        let early_termination_threshold = limit * 3; // Collect 3x the limit then stop

        // Search each segment using M05's extension trait
        for segment_reader in &self.segments {
            // Check if we can terminate early
            if all_results.len() >= early_termination_threshold && limit < usize::MAX {
                // We have enough candidates, check if we should continue
                // Sort to see if lower segments could have better scores
                all_results.sort_by(|a: &ScoredDocument, b: &ScoredDocument| b.score.partial_cmp(&a.score).unwrap());
                
                // If the worst score in our top-K is good enough, we can stop
                if all_results.len() >= limit {
                    let kth_score = all_results[limit - 1].score;
                    // Simple heuristic: if kth score is > 0.5, probably good enough
                    if kth_score > 0.5 {
                        break; // Early termination
                    }
                }
            }
            
            let reader = segment_reader.read();  // Use READ lock for concurrent reads
            let results = reader.search(query, &matcher)?;
            all_results.extend(results);
        }
        
        // Filter deleted documents
        all_results.retain(|doc| {
            !self.deleted_docs.contains(doc.doc_id.0 as u32)
        });
        
        // Sort and take top K results
        all_results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        
        let total_hits = all_results.len();
        let max_score = all_results.first().map(|h| h.score).unwrap_or(0.0);
        
        // Truncate to limit if specified
        if limit < usize::MAX && all_results.len() > limit {
            all_results.truncate(limit);
        }

        Ok(SearchResults {
            hits: all_results,
            total_hits,
            max_score,
            took_ms: 0,
        })
    }
}