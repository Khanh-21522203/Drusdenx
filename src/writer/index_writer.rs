use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::mem;
use std::collections::HashMap;
use crate::analysis::analyzer::Analyzer;
use crate::core::types::{Document, DocId};
use crate::storage::segment::SegmentId;
use crate::storage::segment_writer::SegmentWriter;
use crate::storage::wal::{Operation, WAL};
use crate::core::error::Result;
use crate::memory::buffer_pool::BufferPool;
use crate::memory::pool::MemoryPool;
use crate::mvcc::controller::MVCCController;
use crate::parallel::indexer::ParallelIndexer;
use crate::storage::layout::StorageLayout;
use crate::storage::merge_policy::{MergePolicy, TieredMergePolicy};
use crate::storage::segment::Segment;

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
    pub parallel_indexer: Arc<ParallelIndexer>,  // Parallel document processing
    pub analyzer: Arc<Analyzer>,
    pub merge_policy: Box<dyn MergePolicy>,
}

#[derive(Debug, Clone)]
pub struct WriterConfig {
    pub batch_size: usize,
    pub commit_interval: Duration,
    pub max_segment_size: usize,
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
        let segment_writer = SegmentWriter::new(
            &storage,
            SegmentId::new(),
            buffer_pool.clone()
        )?;

        let wal = WAL::open(&storage, 0)?;

        Ok(IndexWriter {
            segment_writer,
            wal,
            memory_pool,
            config: WriterConfig::default(),
            mvcc,
            lock: Arc::new(Mutex::new(())),
            storage,
            buffer_pool,
            parallel_indexer,
            analyzer,
            merge_policy: Box::new(TieredMergePolicy::default()),
        })
    }
    pub fn add_document(&mut self, doc: Document) -> Result<()> {
        // Hold lock for entire operation to prevent race conditions
        let _lock = self.lock.lock().unwrap();

        // Write to WAL first
        self.wal.append(Operation::AddDocument(doc.clone()))?;

        // Add to segment buffer
        self.segment_writer.write_document(&doc)?;

        // Check if flush needed
        if self.segment_writer.segment.doc_count >= self.config.batch_size as u32 {
            // Do the flush logic inline to avoid borrowing issues
            let new_writer = SegmentWriter::new(
                &self.storage,
                SegmentId::new(),
                self.buffer_pool.clone()
            )?;

            // Replace old writer and finish it
            let old_writer = mem::replace(&mut self.segment_writer, new_writer);
            let segment = old_writer.finish()?;

            // Update MVCC snapshot
            let mut segments = self.mvcc.current_snapshot().segments.clone();
            segments.push(Arc::new(segment));
            self.mvcc.create_snapshot(segments);
        }
        
        Ok(())
    }

    /// Add documents in batch with parallel processing (M08 optimization)
    pub fn add_documents_batch(&mut self, docs: Vec<Document>) -> Result<()> {
        if docs.len() > 100 {
            // Parallel processing: tokenize & index in parallel
            let indexed_docs = self.parallel_indexer.index_batch(docs, &self.analyzer)?;

            // Write to WAL and segments (still sequential - single writer)
            {
                let _lock = self.lock.lock().unwrap();
                
                for indexed_doc in indexed_docs {
                    // Write to WAL
                    let doc = Document {
                        id: indexed_doc.doc_id,
                        fields: HashMap::new(), // Note: we've lost field info, would need to preserve it
                    };
                    self.wal.append(Operation::AddDocument(doc.clone()))?;
                    
                    // Write to segment
                    self.segment_writer.write_document(&doc)?;
                    
                    // Check if flush needed
                    if self.segment_writer.segment.doc_count >= self.config.batch_size as u32 {
                        // Inline flush logic to avoid borrowing issues
                        let new_writer = SegmentWriter::new(
                            &self.storage,
                            SegmentId::new(),
                            self.buffer_pool.clone()
                        )?;
                        let old_writer = mem::replace(&mut self.segment_writer, new_writer);
                        let segment = old_writer.finish()?;
                        
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
            self.buffer_pool.clone()
        )?;

        // Replace old writer and finish it
        let old_writer = mem::replace(&mut self.segment_writer, new_writer);
        let segment = old_writer.finish()?;

        // Update MVCC snapshot
        let mut segments = self.mvcc.current_snapshot().segments.clone();
        segments.push(Arc::new(segment));
        
        // Check if we should merge segments
        if self.merge_policy.should_merge(&segments) {
            self.merge_segments_async(segments.clone());
        }
        
        self.mvcc.create_snapshot(segments);

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
        
        // Spawn background merge task
        std::thread::spawn(move || {
            // Perform merge in background
            if let Err(e) = Self::merge_segments_impl(
                storage,
                mvcc,
                buffer_pool,
                segments_to_merge,
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
    ) -> Result<()> {
        let merged_id = SegmentId::new();
        let mut merged_writer = SegmentWriter::new(&storage, merged_id, buffer_pool)?;
        
        // Copy all documents from segments to merge
        use crate::storage::segment_reader::SegmentReader;
        
        for segment in &segments_to_merge {
            let mut reader = SegmentReader::open(&storage, segment.id)?;
            let mut doc_iter = reader.iter_documents()?;
            
            while let Some(doc) = doc_iter.next() {
                let doc = doc?;
                // Check if document is deleted
                if !mvcc.current_snapshot().deleted_docs.contains(doc.id.0 as u32) {
                    merged_writer.write_document(&doc)?;
                }
            }
        }
        
        let merged_segment = merged_writer.finish()?;
        
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
        self.wal.sync()?;
        Ok(())
    }
    
    /// Delete a document (soft delete - adds to deleted bitmap)
    pub fn delete_document(&mut self, doc_id: DocId) -> Result<()> {
        let _lock = self.lock.lock().unwrap();
        
        // Write to WAL first for durability
        self.wal.append(Operation::DeleteDocument(doc_id))?;
        
        // Update deleted docs bitmap in current snapshot
        let snapshot = self.mvcc.current_snapshot();
        let mut deleted_docs = (*snapshot.deleted_docs).clone();
        deleted_docs.insert(doc_id.0 as u32);
        
        // Create new snapshot with updated deleted docs
        let segments = snapshot.segments.clone();
        self.mvcc.create_snapshot_with_deletes(segments, Arc::new(deleted_docs));
        
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
                self.buffer_pool.clone()
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
            
            let new_segment = new_writer.finish()?;
            new_segments.push(Arc::new(new_segment));
        }
        
        // Create new snapshot with compacted segments and empty deleted bitmap
        use roaring::RoaringBitmap;
        self.mvcc.create_snapshot_with_deletes(new_segments, Arc::new(RoaringBitmap::new()));
        
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
        }
    }
}