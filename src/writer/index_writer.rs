use std::time::Duration;
use std::sync::{Arc, Mutex};
use crate::core::types::Document;
use crate::storage::segment::SegmentId;
use crate::storage::segment_writer::SegmentWriter;
use crate::storage::wal::{Operation, WAL};
use crate::core::error::Result;
use crate::memory::pool::MemoryPool;
use crate::mvcc::controller::MVCCController;
use crate::storage::layout::StorageLayout;

/// Single writer with MVCC
pub struct IndexWriter {
    pub segment_writer: SegmentWriter,
    pub wal: WAL,
    pub memory_pool: MemoryPool,
    pub config: WriterConfig,
    pub mvcc: Arc<MVCCController>,
    pub lock: Arc<Mutex<()>>, // Single writer lock
    pub storage: Arc<StorageLayout>,
}

#[derive(Debug, Clone)]
pub struct WriterConfig {
    pub batch_size: usize,
    pub commit_interval: Duration,
    pub max_segment_size: usize,
}

impl IndexWriter {
    pub fn add_document(&mut self, doc: Document) -> Result<()> {
        let _lock = self.lock.lock().unwrap();

        // Write to WAL first
        self.wal.append(Operation::AddDocument(doc.clone()))?;

        // Add to segment buffer
        self.segment_writer.write_document(&doc)?;

        // Check if flush needed
        let should_flush = self.segment_writer.segment.doc_count >= self.config.batch_size as u32;

        drop(_lock);

        if should_flush {
            self.flush()?;
        }

        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        // Create new segment writer (doesn't need lock - no reader visibility)
        let new_writer = SegmentWriter::new(&self.storage, SegmentId::new())?;

        // Replace old writer and finish it (takes ownership)
        let old_writer = std::mem::replace(&mut self.segment_writer, new_writer);
        let segment = old_writer.finish()?;

        // Update MVCC snapshot
        let mut segments = self.mvcc.current_snapshot().segments.clone();
        segments.push(Arc::new(segment));
        self.mvcc.create_snapshot(segments);

        Ok(())
    }

    pub fn commit(&mut self) -> Result<()> {
        self.flush()?;
        self.wal.sync()?;
        Ok(())
    }
}

impl Default for WriterConfig {
    fn default() -> Self {
        WriterConfig {
            batch_size: 1000,
            commit_interval: Duration::from_secs(5),
            max_segment_size: 100_000,
        }
    }
}