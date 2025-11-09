use std::io::{Write, Seek, SeekFrom};
use std::fs::File;
use chrono::Utc;
use crc32fast::Hasher;
use std::cmp;
use std::sync::Arc;
use crate::compression::compress::{CompressedBlock, CompressionType};
use crate::core::types::{DocId, Document};
use crate::storage::layout::StorageLayout;
use crate::storage::segment::{Segment, SegmentHeader, SegmentId, SegmentMetadata};
use crate::core::error::Result;
use crate::memory::buffer_pool::BufferPool;

pub struct SegmentWriter {
    pub segment: Segment,
    pub buffer: Vec<u8>,
    pub file: File,
    pub hasher: Hasher,
    pub buffer_pool: Arc<BufferPool>,
}

impl SegmentWriter {
    pub fn new(
        storage: &StorageLayout,
        segment_id: SegmentId,
        buffer_pool: Arc<BufferPool>
    ) -> Result<Self> {
        let path = storage.segment_path(&segment_id);
        let file = File::create(path)?;

        Ok(SegmentWriter {
            segment: Segment {
                id: segment_id,
                doc_count: 0,
                metadata: SegmentMetadata {
                    created_at: Utc::now(),
                    size_bytes: 0,
                    min_doc_id: DocId(u64::MAX),
                    max_doc_id: DocId(0),
                },
            },
            buffer: Vec::with_capacity(1024 * 1024), // 1MB buffer
            file,
            hasher: Hasher::new(),
            buffer_pool,
        })
    }

    /// Write document with compression (M08 optimization)
    pub fn write_document(&mut self, doc: &Document) -> Result<u64> {
        // Serialize document
        let data = bincode::serialize(doc)?;

        let compressed = CompressedBlock::compress(&data, CompressionType::LZ4)?;

        let mut pooled_buffer = self.buffer_pool.get(compressed.data.len());

        // Write length prefix (compressed size)
        let len = compressed.data.len() as u32;
        pooled_buffer.extend_from_slice(&len.to_le_bytes());
        pooled_buffer.extend_from_slice(&compressed.data);

        // Add to internal buffer
        let offset = self.buffer.len() as u64;
        self.buffer.extend_from_slice(&pooled_buffer);

        self.buffer_pool.return_buffer(pooled_buffer);

        // Update metadata
        self.segment.doc_count += 1;
        self.segment.metadata.min_doc_id =
            DocId(cmp::min(self.segment.metadata.min_doc_id.0, doc.id.0));
        self.segment.metadata.max_doc_id =
            DocId(cmp::max(self.segment.metadata.max_doc_id.0, doc.id.0));

        // Flush if buffer is large
        if self.buffer.len() > 1024 * 1024 {
            self.flush()?;
        }

        Ok(offset)
    }

    pub fn flush(&mut self) -> Result<()> {
        if !self.buffer.is_empty() {
            self.hasher.update(&self.buffer);
            self.file.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }

    // [ HEADER (doc_count, checksum, metadata) ] <- byte 0
    // [ DOCUMENT 1 ]
    // [ DOCUMENT 2 ]
    // [ DOCUMENT 3 ]
    pub fn finish(mut self) -> Result<Segment> {
        self.flush()?;

        // Write header at the beginning
        self.file.seek(SeekFrom::Start(0))?;
        let mut header = SegmentHeader::new(self.segment.doc_count);
        header.checksum = self.hasher.finalize();

        let header_data = bincode::serialize(&header)?;
        self.file.write_all(&header_data)?;

        self.file.sync_all()?;

        // Update size
        self.segment.metadata.size_bytes = self.file.metadata()?.len() as usize;

        Ok(self.segment)
    }
}
