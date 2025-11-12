use std::io::{Write, Seek, SeekFrom};
use std::fs::File;
use chrono::Utc;
use crc32fast::Hasher;
use std::cmp;
use std::sync::Arc;
use std::collections::HashMap;
use crate::compression::compress::{CompressedBlock, CompressionType};
use crate::core::types::{DocId, Document};
use crate::storage::layout::StorageLayout;
use crate::storage::segment::{Segment, SegmentHeader, SegmentId, SegmentMetadata};
use crate::core::error::Result;
use crate::memory::buffer_pool::BufferPool;
use crate::index::inverted::Term;
use crate::index::posting::Posting;

pub struct SegmentWriter {
    pub segment: Segment,
    pub buffer: Vec<u8>,
    pub file: File,
    pub hasher: Hasher,
    pub buffer_pool: Arc<BufferPool>,
    pub inverted_index: HashMap<Term, Vec<Posting>>,  // In-memory index buffer
}

impl SegmentWriter {
    pub fn new(
        storage: &StorageLayout,
        segment_id: SegmentId,
        buffer_pool: Arc<BufferPool>
    ) -> Result<Self> {
        let path = storage.segment_path(&segment_id);
        let mut file = File::create(path)?;
        
        // Write placeholder header to reserve space (will be updated in finish())
        let placeholder_header = SegmentHeader::new(0);
        let header_data = bincode::serialize(&placeholder_header)?;
        file.write_all(&header_data)?;
        file.flush()?;

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
            inverted_index: HashMap::new(),
        })
    }
    
    /// Add inverted index entry
    pub fn add_index_entry(&mut self, term: Term, posting: Posting) {
        self.inverted_index
            .entry(term)
            .or_insert_with(Vec::new)
            .push(posting);
    }

    /// Write document with compression (M08 optimization)
    pub fn write_document(&mut self, doc: &Document) -> Result<u64> {
        // Serialize document
        let data = bincode::serialize(doc)?;

        let compressed = CompressedBlock::compress(&data, CompressionType::LZ4)?;
        
        // Serialize the entire CompressedBlock (includes original_size metadata)
        let compressed_block_data = bincode::serialize(&compressed)?;

        let mut pooled_buffer = self.buffer_pool.get(compressed_block_data.len());
        pooled_buffer.clear(); // CRITICAL: Clear the pooled buffer before use!

        // Write length prefix (serialized CompressedBlock size)
        let len = compressed_block_data.len() as u32;
        pooled_buffer.extend_from_slice(&len.to_le_bytes());
        pooled_buffer.extend_from_slice(&compressed_block_data);

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
    pub fn finish(mut self, storage: &StorageLayout) -> Result<Segment> {
        self.flush()?;

        // Calculate checksum before consuming hasher
        let checksum = self.hasher.clone().finalize();

        // Write header at the beginning
        self.file.seek(SeekFrom::Start(0))?;
        let mut header = SegmentHeader::new(self.segment.doc_count);
        header.checksum = checksum;

        let header_data = bincode::serialize(&header)?;
        self.file.write_all(&header_data)?;

        self.file.sync_all()?;

        // Update size
        self.segment.metadata.size_bytes = self.file.metadata()?.len() as usize;
        
        // Write inverted index to separate file (.idx)
        if !self.inverted_index.is_empty() {
            self.write_inverted_index(storage)?;
        }

        Ok(self.segment)
    }
    
    /// Write inverted index to disk (.idx file)
    fn write_inverted_index(&self, storage: &StorageLayout) -> Result<()> {
        // Create index file path in idx/ folder
        let index_path = storage.index_path(&self.segment.id);
        let mut index_file = File::create(index_path)?;
        
        // Sort postings by doc_id for each term
        let mut sorted_index = self.inverted_index.clone();
        for postings in sorted_index.values_mut() {
            postings.sort_by_key(|p| p.doc_id);
        }
        
        // Serialize and compress inverted index
        let index_data = bincode::serialize(&sorted_index)?;
        let compressed = CompressedBlock::compress(&index_data, CompressionType::LZ4)?;
        
        // Write the entire CompressedBlock (including metadata) to file
        let compressed_block_data = bincode::serialize(&compressed)?;
        index_file.write_all(&compressed_block_data)?;
        index_file.sync_all()?;
        
        Ok(())
    }
}
