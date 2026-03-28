use std::sync::Arc;
use crate::core::error::Result;
use crate::core::types::{DocId, Document};
use crate::query::ast::Query;
use crate::query::matcher::{DocumentMatcher, SegmentSearch};
use crate::reader::segment_opener::{SegmentOpener, SegmentRead};
use crate::search::results::ScoredDocument;
use crate::storage::layout::StorageLayout;
use crate::storage::segment::SegmentId;
use crate::storage::segment_reader::SegmentReader;

/// Production adapter: opens segment files from disk using `SegmentReader::open`.
pub struct DiskSegmentOpener {
    pub layout: Arc<StorageLayout>,
}

impl DiskSegmentOpener {
    pub fn new(layout: Arc<StorageLayout>) -> Self {
        DiskSegmentOpener { layout }
    }
}

impl SegmentOpener for DiskSegmentOpener {
    type Reader = DiskSegmentRead;

    fn open(&self, id: SegmentId) -> Result<DiskSegmentRead> {
        let reader = SegmentReader::open(&self.layout, id)?;
        Ok(DiskSegmentRead { inner: reader })
    }
}

/// Wrapper around `SegmentReader` that implements `SegmentRead`.
pub struct DiskSegmentRead {
    pub inner: SegmentReader,
}

impl SegmentRead for DiskSegmentRead {
    fn segment_id(&self) -> SegmentId {
        self.inner.segment_id
    }

    fn doc_count(&self) -> u32 {
        self.inner.header.doc_count
    }

    fn search(&self, query: &Query, matcher: &DocumentMatcher) -> Result<Vec<ScoredDocument>> {
        self.inner.search(query, matcher)
    }

    fn get_document(&self, doc_id: DocId) -> Result<Option<Document>> {
        self.inner.get_document(doc_id)
    }
}
