use crate::analysis::analyzer::Analyzer;
use crate::compression::compress::CompressionType;
use crate::core::config::MergePolicyType;
use crate::core::error::{Error, ErrorKind, Result};
use crate::core::types::{DocId, Document};
use crate::memory::buffer_pool::BufferPool;
use crate::memory::pool::MemoryPool;
use crate::mvcc::controller::MVCCController;
use crate::parallel::indexer::ParallelIndexer;
use crate::storage::layout::StorageLayout;
use crate::storage::merge_policy::{LogStructuredMergePolicy, MergePolicy, TieredMergePolicy};
use crate::storage::segment::Segment;
use crate::storage::segment::SegmentId;
use crate::storage::segment_writer::SegmentWriter;
use crate::storage::wal::{Operation, WAL};
use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Single writer with MVCC
pub struct IndexWriter {
    pub segment_writer: SegmentWriter,
    pub wal: WAL,
    pub memory_pool: MemoryPool,
    pub config: WriterConfig,
    pub mvcc: Arc<MVCCController>,
    pub lock: Arc<Mutex<()>>, // Single writer lock
    pub storage: Arc<StorageLayout>,
    pub buffer_pool: Arc<BufferPool>,
    pub parallel_indexer: Arc<ParallelIndexer>, // Parallel document processing
    pub analyzer: Arc<Analyzer>,
    pub merge_policy: Box<dyn MergePolicy>,
}

#[derive(Debug, Clone)]
pub struct WriterConfig {
    pub batch_size: usize,
    pub commit_interval: Duration,
    pub max_segment_size: usize,
    pub compression: CompressionType,
}

impl IndexWriter {
    pub fn new(
        storage: Arc<StorageLayout>,
        mvcc: Arc<MVCCController>,
        memory_pool: MemoryPool,
        buffer_pool: Arc<BufferPool>,
        parallel_indexer: Arc<ParallelIndexer>,
        analyzer: Arc<Analyzer>,
    ) -> Result<Self> {
        Self::new_with_merge_policy(
            storage,
            mvcc,
            memory_pool,
            buffer_pool,
            parallel_indexer,
            analyzer,
            MergePolicyType::Tiered, // Default
            CompressionType::LZ4,
        )
    }

    pub fn new_with_merge_policy(
        storage: Arc<StorageLayout>,
        mvcc: Arc<MVCCController>,
        memory_pool: MemoryPool,
        buffer_pool: Arc<BufferPool>,
        parallel_indexer: Arc<ParallelIndexer>,
        analyzer: Arc<Analyzer>,
        merge_policy_type: MergePolicyType,
        compression: CompressionType,
    ) -> Result<Self> {
        let segment_writer =
            SegmentWriter::new(&storage, SegmentId::new(), buffer_pool.clone(), compression)?;

        let wal = WAL::open(&storage, 0)?;

        let merge_policy: Box<dyn MergePolicy> = match merge_policy_type {
            MergePolicyType::Tiered => Box::new(TieredMergePolicy::default()),
            MergePolicyType::LogStructured => Box::new(LogStructuredMergePolicy::default()),
        };

        Ok(IndexWriter {
            segment_writer,
            wal,
            memory_pool,
            config: WriterConfig {
                compression,
                ..WriterConfig::default()
            },
            mvcc,
            lock: Arc::new(Mutex::new(())),
            storage,
            buffer_pool,
            parallel_indexer,
            analyzer,
            merge_policy,
        })
    }
    pub fn add_document(&mut self, doc: Document) -> Result<()> {
        self.add_document_internal(doc, true)
    }

    fn add_document_internal(&mut self, doc: Document, write_wal: bool) -> Result<()> {
        // Hold lock for entire operation to prevent race conditions
        let _lock = self.lock.lock().unwrap();

        // Index the document (tokenize and analyze)
        let indexed_docs = self
            .parallel_indexer
            .index_batch(vec![doc.clone()], &self.analyzer)?;

        // Write to WAL first
        if write_wal {
            self.wal.append(Operation::AddDocument(doc.clone()))?;
        }

        // Add to segment buffer (DATA)
        self.segment_writer.write_document(&doc)?;

        // Add to inverted index (INDEX)
        if let Some(indexed_doc) = indexed_docs.first() {
            // Create posting for this document
            use crate::index::posting::Posting;
            let mut term_positions: HashMap<_, Vec<usize>> = HashMap::new();

            for (pos, token) in indexed_doc.tokens.iter().enumerate() {
                term_positions
                    .entry(token.text.clone())
                    .or_insert_with(Vec::new)
                    .push(pos);
            }

            for (term_text, positions) in term_positions {
                let term = crate::index::inverted::Term::new(&term_text);
                let posting = Posting {
                    doc_id: doc.id,
                    term_freq: positions.len() as u32,
                    positions: positions.into_iter().map(|p| p as u32).collect(),
                    field_norm: 1.0 / (indexed_doc.terms.len() as f32).sqrt(),
                };
                self.segment_writer.add_index_entry(term, posting);
            }
        }

        // Check if flush needed
        if self.segment_writer.segment.doc_count >= self.config.batch_size as u32 {
            // Do the flush logic inline to avoid borrowing issues
            let new_writer = SegmentWriter::new(
                &self.storage,
                SegmentId::new(),
                self.buffer_pool.clone(),
                self.config.compression,
            )?;

            // Replace old writer and finish it
            let old_writer = mem::replace(&mut self.segment_writer, new_writer);
            let segment = old_writer.finish(&self.storage)?; // Pass storage reference

            // Only add segment if it has documents
            if segment.doc_count > 0 {
                // Update MVCC snapshot
                let mut segments = self.mvcc.current_snapshot().segments.clone();
                segments.push(Arc::new(segment));
                self.mvcc.create_snapshot(segments);
            }
        }

        Ok(())
    }

    /// Add documents in batch with parallel processing (M08 optimization)
    pub fn add_documents_batch(&mut self, docs: Vec<Document>) -> Result<()> {
        if docs.len() > 100 {
            // Parallel processing: tokenize & index in parallel
            let indexed_docs = self
                .parallel_indexer
                .index_batch(docs.clone(), &self.analyzer)?;
            if indexed_docs.len() != docs.len() {
                return Err(Error::new(
                    ErrorKind::InvalidState,
                    "Indexed document count does not match input batch size".to_string(),
                ));
            }

            // Write to WAL and segments (still sequential - single writer)
            {
                let _lock = self.lock.lock().unwrap();

                for (doc, indexed_doc) in docs.into_iter().zip(indexed_docs.into_iter()) {
                    if indexed_doc.doc_id != doc.id {
                        return Err(Error::new(
                            ErrorKind::InvalidState,
                            "Indexed document id does not match source document id".to_string(),
                        ));
                    }

                    // Write to WAL
                    self.wal.append(Operation::AddDocument(doc.clone()))?;

                    // Write to segment
                    self.segment_writer.write_document(&doc)?;

                    // Check if flush needed
                    if self.segment_writer.segment.doc_count >= self.config.batch_size as u32 {
                        // Inline flush logic to avoid borrowing issues
                        let new_writer = SegmentWriter::new(
                            &self.storage,
                            SegmentId::new(),
                            self.buffer_pool.clone(),
                            self.config.compression,
                        )?;
                        let old_writer = mem::replace(&mut self.segment_writer, new_writer);
                        let segment = old_writer.finish(&self.storage)?;

                        let mut segments = self.mvcc.current_snapshot().segments.clone();
                        segments.push(Arc::new(segment));

                        if self.merge_policy.should_merge(&segments) {
                            // Note: Can't call async merge while holding lock
                            // Would need to refactor to handle this differently
                        }

                        self.mvcc.create_snapshot(segments);
                    }
                }
            } // Lock is dropped here
        } else {
            // Sequential for small batches (avoid parallelization overhead)
            for doc in docs {
                self.add_document(doc)?;
            }
        }

        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        // Acquire lock before flushing to prevent concurrent modifications
        let _lock = self.lock.lock().unwrap();

        // Do flush inline to avoid borrowing issues
        let new_writer = SegmentWriter::new(
            &self.storage,
            SegmentId::new(),
            self.buffer_pool.clone(),
            self.config.compression,
        )?;

        // Replace old writer and finish it
        let old_writer = mem::replace(&mut self.segment_writer, new_writer);
        let segment = old_writer.finish(&self.storage)?;

        // Only add segment if it has documents (skip empty segments)
        if segment.doc_count > 0 {
            // Update MVCC snapshot
            let mut segments = self.mvcc.current_snapshot().segments.clone();
            segments.push(Arc::new(segment));

            // Check if we should merge segments
            if self.merge_policy.should_merge(&segments) {
                self.merge_segments_async(segments.clone());
            }

            self.mvcc.create_snapshot(segments);
        }

        Ok(())
    }

    /// Merge segments based on merge policy (runs asynchronously)
    fn merge_segments_async(&self, segments: Vec<Arc<Segment>>) {
        let segments_to_merge = self.merge_policy.select_segments_to_merge(&segments);

        if segments_to_merge.is_empty() {
            return;
        }

        // Clone required data for async operation
        let storage = self.storage.clone();
        let mvcc = self.mvcc.clone();
        let buffer_pool = self.buffer_pool.clone();
        let compression = self.config.compression;

        // Spawn background merge task
        std::thread::spawn(move || {
            // Perform merge in background
            if let Err(e) = Self::merge_segments_impl(
                storage,
                mvcc,
                buffer_pool,
                segments_to_merge,
                compression,
            ) {
                eprintln!("Background merge failed: {}", e);
            }
        });
    }

    /// Implementation of segment merging
    fn merge_segments_impl(
        storage: Arc<StorageLayout>,
        mvcc: Arc<MVCCController>,
        buffer_pool: Arc<BufferPool>,
        segments_to_merge: Vec<Arc<Segment>>,
        compression: CompressionType,
    ) -> Result<()> {
        let merged_id = SegmentId::new();
        let mut merged_writer = SegmentWriter::new(&storage, merged_id, buffer_pool, compression)?;

        // Copy all documents from segments to merge
        use crate::storage::segment_reader::SegmentReader;

        for segment in &segments_to_merge {
            let mut reader = SegmentReader::open(&storage, segment.id)?;
            let mut doc_iter = reader.iter_documents()?;

            while let Some(doc) = doc_iter.next() {
                let doc = doc?;
                // Check if document is deleted
                if !mvcc
                    .current_snapshot()
                    .deleted_docs
                    .contains(doc.id.0 as u32)
                {
                    merged_writer.write_document(&doc)?;
                }
            }
        }

        let merged_segment = merged_writer.finish(&storage)?;

        // Update snapshot with merged segment
        let current_snapshot = mvcc.current_snapshot();
        let mut new_segments = Vec::new();

        // Keep segments not being merged
        for seg in &current_snapshot.segments {
            if !segments_to_merge.iter().any(|s| s.id == seg.id) {
                new_segments.push(seg.clone());
            }
        }

        // Add the merged segment
        new_segments.push(Arc::new(merged_segment));

        // Create new snapshot
        mvcc.create_snapshot(new_segments);

        Ok(())
    }

    pub fn commit(&mut self) -> Result<()> {
        self.flush()?;
        self.wal.append(Operation::Commit)?;
        self.wal.sync()?;
        Ok(())
    }

    /// Apply WAL operation during recovery without re-appending to WAL.
    pub(crate) fn apply_recovered_operation(&mut self, operation: Operation) -> Result<()> {
        match operation {
            Operation::AddDocument(doc) => self.add_document_internal(doc, false),
            Operation::UpdateDocument(doc) => self.add_document_internal(doc, false),
            Operation::DeleteDocument(doc_id) => self.delete_document_internal(doc_id, false),
            Operation::Commit => self.flush(),
        }
    }

    /// Delete a document (soft delete - adds to deleted bitmap)
    pub fn delete_document(&mut self, doc_id: DocId) -> Result<()> {
        self.delete_document_internal(doc_id, true)
    }

    fn delete_document_internal(&mut self, doc_id: DocId, write_wal: bool) -> Result<()> {
        let _lock = self.lock.lock().unwrap();

        // Write to WAL first for durability
        if write_wal {
            self.wal.append(Operation::DeleteDocument(doc_id))?;
        }

        // Update deleted docs bitmap in current snapshot
        let snapshot = self.mvcc.current_snapshot();
        let mut deleted_docs = (*snapshot.deleted_docs).clone();
        deleted_docs.insert(doc_id.0 as u32);

        // Create new snapshot with updated deleted docs
        let segments = snapshot.segments.clone();
        self.mvcc
            .create_snapshot_with_deletes(segments, Arc::new(deleted_docs));

        Ok(())
    }

    /// Compact segments to physically remove deleted documents
    /// Creates new segments without deleted documents
    pub fn compact(&mut self) -> Result<()> {
        let _lock = self.lock.lock().unwrap();

        let snapshot = self.mvcc.current_snapshot();
        let deleted_docs = snapshot.deleted_docs.clone();

        if deleted_docs.is_empty() {
            // No deleted documents, nothing to compact
            return Ok(());
        }

        // Create new compacted segments
        let mut new_segments = Vec::new();

        for segment in &snapshot.segments {
            // Check if this segment has any deleted documents
            // We check if any document ID in the deleted bitmap might be in this segment
            // For simplicity, we'll process segments with any deletes in the snapshot
            let segment_has_deletes = !snapshot.deleted_docs.is_empty();

            if !segment_has_deletes {
                // No deletes in this segment, keep it as-is
                new_segments.push(segment.clone());
                continue;
            }

            // Create new segment without deleted documents
            let new_segment_id = SegmentId::new();
            let mut new_writer = SegmentWriter::new(
                &self.storage,
                new_segment_id,
                self.buffer_pool.clone(),
                self.config.compression,
            )?;

            // Copy non-deleted documents to new segment
            use crate::storage::segment_reader::SegmentReader;
            let mut reader = SegmentReader::open(&self.storage, segment.id)?;
            let mut doc_iter = reader.iter_documents()?;

            while let Some(doc) = doc_iter.next() {
                let doc = doc?;
                // Skip deleted documents
                if !snapshot.deleted_docs.contains(doc.id.0 as u32) {
                    new_writer.write_document(&doc)?;
                }
            }

            let new_segment = new_writer.finish(&self.storage)?;
            new_segments.push(Arc::new(new_segment));
        }

        // Create new snapshot with compacted segments and empty deleted bitmap
        use roaring::RoaringBitmap;
        self.mvcc
            .create_snapshot_with_deletes(new_segments, Arc::new(RoaringBitmap::new()));

        // Write compaction to WAL
        self.wal.append(Operation::Commit)?;
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
            compression: CompressionType::LZ4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compression::compress::CompressedBlock;
    use crate::core::types::FieldValue;
    use crate::storage::segment::SegmentHeader;
    use std::fs::File;
    use std::io::Read;

    fn make_doc(id: u64, value: &str) -> Document {
        Document {
            id: DocId(id),
            fields: HashMap::from([("content".to_string(), FieldValue::Text(value.to_string()))]),
        }
    }

    fn make_writer(
        storage: Arc<StorageLayout>,
        mvcc: Arc<MVCCController>,
        compression: CompressionType,
    ) -> IndexWriter {
        IndexWriter::new_with_merge_policy(
            storage,
            mvcc,
            MemoryPool::new(8, 1024 * 1024),
            Arc::new(BufferPool::new(4 * 1024 * 1024)),
            Arc::new(ParallelIndexer::new(2)),
            Arc::new(Analyzer::standard_english()),
            MergePolicyType::Tiered,
            compression,
        )
        .unwrap()
    }

    #[test]
    fn add_documents_batch_preserves_fields_for_large_batches() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(StorageLayout::new(temp_dir.path().to_path_buf()).unwrap());
        let mvcc = Arc::new(MVCCController::new());
        let mut writer = make_writer(storage.clone(), mvcc.clone(), CompressionType::LZ4);

        let docs: Vec<Document> = (0..101)
            .map(|i| make_doc(i as u64, &format!("value-{}", i)))
            .collect();

        writer.add_documents_batch(docs.clone()).unwrap();
        writer.commit().unwrap();

        let snapshot = mvcc.current_snapshot();
        assert_eq!(snapshot.segments.len(), 1);
        let mut seen: HashMap<DocId, Document> = HashMap::new();

        let mut segment_file = File::open(storage.segment_path(&snapshot.segments[0].id)).unwrap();
        let header: SegmentHeader = bincode::deserialize_from(&mut segment_file).unwrap();
        for _ in 0..header.doc_count {
            let mut len_buf = [0u8; 4];
            segment_file.read_exact(&mut len_buf).unwrap();
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut block_buf = vec![0u8; len];
            segment_file.read_exact(&mut block_buf).unwrap();
            let block: CompressedBlock = bincode::deserialize(&block_buf).unwrap();
            let raw_doc = block.decompress().unwrap();
            let doc: Document = bincode::deserialize(&raw_doc).unwrap();
            seen.insert(doc.id, doc);
        }

        for doc in docs {
            let persisted = seen.get(&doc.id).unwrap();
            assert_eq!(persisted.fields, doc.fields);
        }
    }

    #[test]
    fn segment_and_index_files_use_writer_compression_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(StorageLayout::new(temp_dir.path().to_path_buf()).unwrap());
        let mvcc = Arc::new(MVCCController::new());
        let mut writer = make_writer(storage.clone(), mvcc.clone(), CompressionType::Zstd);

        writer.add_document(make_doc(1, "zstd document")).unwrap();
        writer.commit().unwrap();

        let snapshot = mvcc.current_snapshot();
        let segment = snapshot.segments.first().unwrap();

        let mut segment_file = File::open(storage.segment_path(&segment.id)).unwrap();
        let _header: SegmentHeader = bincode::deserialize_from(&mut segment_file).unwrap();
        let mut len_buf = [0u8; 4];
        segment_file.read_exact(&mut len_buf).unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut block_buf = vec![0u8; len];
        segment_file.read_exact(&mut block_buf).unwrap();
        let segment_block: CompressedBlock = bincode::deserialize(&block_buf).unwrap();
        assert!(matches!(segment_block.compression, CompressionType::Zstd));

        let idx_data = std::fs::read(storage.index_path(&segment.id)).unwrap();
        let idx_block: CompressedBlock = bincode::deserialize(&idx_data).unwrap();
        assert!(matches!(idx_block.compression, CompressionType::Zstd));
    }
}
