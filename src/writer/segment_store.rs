use std::sync::Arc;
use crate::core::error::Result;
use crate::core::types::Document;
use crate::index::inverted::Term;
use crate::index::posting::Posting;
use crate::storage::segment::Segment;

/// Port: writeable sink for a single segment being built.
/// Consumed (via `Box<Self>`) when the segment is finalized.
pub trait SegmentSink: Send {
    fn write_document(&mut self, doc: &Document) -> Result<()>;
    fn add_index_entry(&mut self, term: Term, posting: Posting);
    fn doc_count(&self) -> u32;
    /// Finalize and persist the segment, consuming the sink.
    fn finish(self: Box<Self>) -> Result<Arc<Segment>>;
}

/// Port: factory for creating new segment sinks.
pub trait SegmentStore: Send + Sync {
    fn create_sink(&self) -> Result<Box<dyn SegmentSink>>;
    fn iter_documents(
        &self,
        segment: &Arc<Segment>,
        deleted: &roaring::RoaringBitmap,
    ) -> Result<Box<dyn Iterator<Item = Result<Document>>>>;
}
