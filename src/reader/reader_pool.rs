use std::sync::Arc;
use parking_lot::RwLock;
use roaring::RoaringBitmap;
use crate::mvcc::controller::{MVCCController, Snapshot};
use crate::storage::segment_reader::SegmentReader;
use crate::core::error::Result;
use crate::index::inverted::InvertedIndex;
use crate::query::ast::Query;
use crate::query::matcher::{DocumentMatcher, SegmentSearch};
use crate::search::results::SearchResults;
use crate::storage::layout::StorageLayout;

/// Pool of index readers
pub struct ReaderPool {
    pub readers: Arc<RwLock<Vec<Arc<IndexReader>>>>,
    pub mvcc: Arc<MVCCController>,
    pub max_readers: usize,
    pub storage: Arc<StorageLayout>,
    pub index: Arc<InvertedIndex>,
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
        }
    }
    pub fn get_reader(&self) -> Result<Arc<IndexReader>> {
        let snapshot = self.mvcc.current_snapshot();

        let deleted_docs = snapshot.deleted_docs.clone();

        // Create reader for snapshot
        let mut segment_readers = Vec::new();
        for segment in &snapshot.segments {
            // TODO: Add to M02 SegmentReader:
            // pub fn open(storage: &StorageLayout, segment_id: SegmentId) -> Result<Self>
            // For now, use placeholder
            let reader = SegmentReader::open(&self.storage, segment.id)?;
            segment_readers.push(Arc::new(RwLock::new(reader)));
        }

        Ok(Arc::new(IndexReader {
            snapshot,
            segments: segment_readers,
            deleted_docs,
            index: self.index.clone(),
        }))
    }
}

impl IndexReader {
    pub fn search(&self, query: &Query) -> Result<SearchResults> {
        let matcher = DocumentMatcher::new(self.index.clone());
        let mut all_results = Vec::new();

        // Search each segment using M05's extension trait
        for segment_reader in &self.segments {
            let mut reader = segment_reader.write();
            let results = reader.search(query, &matcher)?;
            all_results.extend(results);
        }
        // Filter deleted documents
        all_results.retain(|doc| {
            !self.deleted_docs.contains(doc.doc_id.0 as u32)
        });

        // Calculate values BEFORE moving all_results
        let total_hits = all_results.len();
        let max_score = all_results.iter()
            .map(|h| h.score)
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);

        Ok(SearchResults {
            hits: all_results,
            total_hits,
            max_score,
            took_ms: 0,
        })
    }
}