use crate::compression::compress::CompressionType;
use crate::core::error::Result;
use crate::core::types::Document;
use crate::index::inverted::Term;
use crate::index::posting::Posting;
use crate::memory::buffer_pool::BufferPool;
use crate::storage::layout::StorageLayout;
use crate::storage::segment::{Segment, SegmentId};
use crate::storage::segment_reader::SegmentReader;
use crate::storage::segment_writer::SegmentWriter;
use crate::writer::segment_store::{SegmentSink, SegmentStore};
use std::sync::Arc;

/// Production segment store backed by real disk files.
pub struct DiskSegmentStore {
    pub storage: Arc<StorageLayout>,
    pub buffer_pool: Arc<BufferPool>,
}

impl DiskSegmentStore {
    pub fn new(storage: Arc<StorageLayout>, buffer_pool: Arc<BufferPool>) -> Self {
        DiskSegmentStore {
            storage,
            buffer_pool,
        }
    }
}

impl SegmentStore for DiskSegmentStore {
    fn create_sink(&self) -> Result<Box<dyn SegmentSink>> {
        let writer = SegmentWriter::new(
            &self.storage,
            SegmentId::new(),
            self.buffer_pool.clone(),
            CompressionType::LZ4,
        )?;
        Ok(Box::new(DiskSegmentSink {
            inner: writer,
            storage: self.storage.clone(),
        }))
    }

    fn iter_documents(
        &self,
        segment: &Arc<Segment>,
        deleted: &roaring::RoaringBitmap,
    ) -> Result<Box<dyn Iterator<Item = Result<Document>>>> {
        let mut reader = SegmentReader::open(&self.storage, segment.id)?;
        let mut docs = Vec::new();
        {
            let mut iter = reader.iter_documents()?;
            while let Some(doc_result) = iter.next() {
                match doc_result {
                    Ok(doc) => {
                        if !deleted.contains(doc.id.0 as u32) {
                            docs.push(Ok(doc));
                        }
                    }
                    Err(e) => docs.push(Err(e)),
                }
            }
        }
        Ok(Box::new(docs.into_iter()))
    }
}

/// Production segment sink backed by a `SegmentWriter`.
pub struct DiskSegmentSink {
    inner: SegmentWriter,
    storage: Arc<StorageLayout>,
}

impl SegmentSink for DiskSegmentSink {
    fn write_document(&mut self, doc: &Document) -> Result<()> {
        self.inner.write_document(doc)?;
        Ok(())
    }

    fn add_index_entry(&mut self, term: Term, posting: Posting) {
        self.inner.add_index_entry(term, posting);
    }

    fn doc_count(&self) -> u32 {
        self.inner.segment.doc_count
    }

    fn finish(self: Box<Self>) -> Result<Arc<Segment>> {
        let segment = self.inner.finish(&self.storage)?;
        Ok(Arc::new(segment))
    }
}
