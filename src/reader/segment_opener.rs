use crate::core::error::Result;
use crate::core::types::{DocId, Document};
use crate::query::ast::Query;
use crate::query::matcher::DocumentMatcher;
use crate::search::results::ScoredDocument;
use crate::storage::segment::SegmentId;

/// Port: defines how to open a segment for reading.
/// Implement this to provide different storage backends (disk, in-memory, etc.)
pub trait SegmentOpener: Send + Sync + 'static {
    type Reader: SegmentRead;
    fn open(&self, id: SegmentId) -> Result<Self::Reader>;
}

/// Port: defines operations available on an open segment reader.
pub trait SegmentRead: Send + Sync {
    fn segment_id(&self) -> SegmentId;
    fn doc_count(&self) -> u32;
    fn search(&self, query: &Query, matcher: &DocumentMatcher) -> Result<Vec<ScoredDocument>>;
    fn get_document(&self, doc_id: DocId) -> Result<Option<Document>>;
}
