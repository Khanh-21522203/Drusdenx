use std::sync::{Arc, Mutex};
use crate::core::types::Document;
use crate::storage::segment::SegmentId;
use crate::storage::segment_writer::SegmentWriter;
use crate::storage::wal::{Operation, WAL};
use crate::core::error::Result;
use crate::memory::buffer_pool::BufferPool;
use crate::storage::layout::StorageLayout;
use crate::storage::segment::Segment;
use std::mem;

/// DataWriter handles WAL and data persistence
pub struct DataWriter {
    pub segment_writer: SegmentWriter,
    pub wal: WAL,
    pub lock: Arc<Mutex<()>>,
    pub storage: Arc<StorageLayout>,
    pub buffer_pool: Arc<BufferPool>,
    pub batch_size: usize,
    pub pending_docs: Vec<Document>,  // Batch buffer
}

impl DataWriter {
    pub fn new(
        storage: Arc<StorageLayout>,
        buffer_pool: Arc<BufferPool>,
        batch_size: usize,
    ) -> Result<Self> {
        let segment_writer = SegmentWriter::new(
            &storage,
            SegmentId::new(),
            buffer_pool.clone()
        )?;

        let wal = WAL::open(&storage, 0)?;

        Ok(DataWriter {
            segment_writer,
            wal,
            lock: Arc::new(Mutex::new(())),
            storage,
            buffer_pool,
            batch_size,
            pending_docs: Vec::with_capacity(100),
        })
    }

    /// Write document to WAL and segment
    pub fn write_document(&mut self, doc: &Document) -> Result<()> {
        let _lock = self.lock.lock().unwrap();

        // Write to WAL first for durability
        self.wal.append(Operation::AddDocument(doc.clone()))?;

        // Write to segment file
        self.segment_writer.write_document(doc)?;

        Ok(())
    }
    
    /// Add document to batch (optimized for bulk writes)
    pub fn add_to_batch(&mut self, doc: Document) {
        self.pending_docs.push(doc);
    }
    
    /// Flush batch - write all pending documents at once
    pub fn flush_batch(&mut self) -> Result<usize> {
        if self.pending_docs.is_empty() {
            return Ok(0);
        }
        
        let _lock = self.lock.lock().unwrap();
        let count = self.pending_docs.len();
        
        // Batch WAL writes
        for doc in &self.pending_docs {
            self.wal.append(Operation::AddDocument(doc.clone()))?;
        }
        
        // Batch segment writes
        for doc in &self.pending_docs {
            self.segment_writer.write_document(doc)?;
        }
        
        self.pending_docs.clear();
        Ok(count)
    }

    /// Check if flush is needed based on batch size
    pub fn should_flush(&self) -> bool {
        self.segment_writer.segment.doc_count >= self.batch_size as u32
    }

    /// Flush current segment and create new one
    pub fn flush(&mut self) -> Result<Segment> {
        let _lock = self.lock.lock().unwrap();

        // Create new writer
        let new_writer = SegmentWriter::new(
            &self.storage,
            SegmentId::new(),
            self.buffer_pool.clone()
        )?;

        // Replace and finish old writer
        let old_writer = mem::replace(&mut self.segment_writer, new_writer);
        let segment = old_writer.finish(&self.storage)?;

        Ok(segment)
    }

    /// Commit WAL
    pub fn commit(&mut self) -> Result<()> {
        self.wal.sync()?;
        Ok(())
    }
}
