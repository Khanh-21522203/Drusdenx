use std::sync::Arc;
use parking_lot::RwLock;
use crate::core::types::{DocId, Document};
use crate::mvcc::controller::Snapshot;
use crate::query::ast::Query;
use crate::search::results::ScoredDocument;
use crate::storage::segment_reader::SegmentReader;
use crate::core::error::Result;
use crate::index::inverted::InvertedIndex;
use crate::query::matcher::{DocumentMatcher, SegmentSearch};
use crate::storage::layout::StorageLayout;

/// Reader for a specific snapshot
pub struct SnapshotReader {
    pub snapshot: Arc<Snapshot>,
    pub segment_readers: Vec<Arc<RwLock<SegmentReader>>>,
    pub index: Arc<InvertedIndex>,
}

impl SnapshotReader {
    pub fn new(snapshot: Arc<Snapshot>, storage: &StorageLayout, index: Arc<InvertedIndex>)
        -> Result<Self> {
        let mut segment_readers = Vec::new();

        for segment in &snapshot.segments {
            // Use SegmentReader::open() from M02
            let reader = SegmentReader::open(storage, segment.id)?;
            segment_readers.push(Arc::new(RwLock::new(reader)));
        }

        Ok(SnapshotReader {
            snapshot,
            segment_readers,
            index,
        })
    }

    pub fn search(&self, query: &Query) -> Result<Vec<ScoredDocument>> {
        let matcher = DocumentMatcher::new(self.index.clone());
        let mut results = Vec::new();

        // Search each segment using M05's extension trait
        for reader in &self.segment_readers {
            let segment = reader.write();
            let segment_results = segment.search(query, &matcher)?;
            results.extend(segment_results);
        }

        // Filter deleted docs
        results.retain(|doc| {
            !self.snapshot.deleted_docs.contains(doc.doc_id.0 as u32)
        });

        Ok(results)
    }

    pub fn get_document(&self, doc_id: DocId) -> Result<Option<Document>> {
        // Check if deleted
        if self.snapshot.deleted_docs.contains(doc_id.0 as u32) {
            return Ok(None);
        }

        // Search in segments
        for reader in &self.segment_readers {
            let segment = reader.write();
            if let Some(doc) = segment.get_document(doc_id)? {
                return Ok(Some(doc));
            }
        }

        Ok(None)
    }
}