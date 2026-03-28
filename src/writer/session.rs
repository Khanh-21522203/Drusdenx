use std::sync::Arc;
use crate::analysis::analyzer::Analyzer;
use crate::core::error::Result;
use crate::core::types::{Document, DocId};
use crate::memory::buffer_pool::BufferPool;
use crate::mvcc::controller::MVCCController;
use crate::parallel::indexer::ParallelIndexer;
use crate::storage::layout::StorageLayout;
use crate::storage::merge_policy::MergePolicy;
use crate::storage::segment::Segment;
use crate::storage::wal::Operation;
use crate::writer::segment_store::{SegmentSink, SegmentStore};
use crate::writer::wal_backend::WriteAheadLog;
use crate::writer::index_writer::WriterConfig;

/// Shared infrastructure. Created once; shared across write sessions.
pub struct WriterContext {
    pub storage: Arc<StorageLayout>,
    pub mvcc: Arc<MVCCController>,
    pub buffer_pool: Arc<BufferPool>,
    pub parallel_indexer: Arc<ParallelIndexer>,
    pub analyzer: Arc<Analyzer>,
    pub merge_policy: Arc<dyn MergePolicy>,
    pub config: WriterConfig,
    // Backend seams (injectable):
    pub segment_store: Arc<dyn SegmentStore>,
}

impl WriterContext {
    /// Open a new write session: allocates a WAL and a SegmentSink.
    pub fn open_session(
        &self,
        wal: Box<dyn WriteAheadLog>,
    ) -> Result<WriteSession> {
        let segment_sink = self.segment_store.create_sink()?;
        Ok(WriteSession {
            ctx: WriterContext {
                storage: self.storage.clone(),
                mvcc: self.mvcc.clone(),
                buffer_pool: self.buffer_pool.clone(),
                parallel_indexer: self.parallel_indexer.clone(),
                analyzer: self.analyzer.clone(),
                merge_policy: self.merge_policy.clone(),
                config: self.config.clone(),
                segment_store: self.segment_store.clone(),
            },
            wal,
            segment_sink,
            pending_deletes: Vec::new(),
        })
    }
}

/// Phase 1: Accumulate writes. WAL is open, segment buffer is in memory.
/// No `commit()` method — must call `flush()` first.
pub struct WriteSession {
    pub ctx: WriterContext,
    pub wal: Box<dyn WriteAheadLog>,
    pub segment_sink: Box<dyn SegmentSink>,
    pub pending_deletes: Vec<DocId>,
}

impl WriteSession {
    pub fn add_document(&mut self, doc: Document) -> Result<DocId> {
        let doc_id = doc.id;
        self.wal.append(Operation::AddDocument(doc.clone()))?;
        self.segment_sink.write_document(&doc)?;
        Ok(doc_id)
    }

    pub fn delete_document(&mut self, id: DocId) -> Result<()> {
        self.wal.append(Operation::DeleteDocument(id))?;
        self.pending_deletes.push(id);
        Ok(())
    }

    pub fn add_documents(&mut self, docs: Vec<Document>) -> Result<Vec<DocId>> {
        let mut ids = Vec::with_capacity(docs.len());
        for doc in docs {
            ids.push(self.add_document(doc)?);
        }
        Ok(ids)
    }

    /// Consumes self → FlushedSession. Cannot add documents after this.
    pub fn flush(self) -> Result<FlushedSession> {
        let doc_count = self.segment_sink.doc_count();
        let finished_segment = if doc_count > 0 {
            Some(self.segment_sink.finish()?)
        } else {
            // Drop the sink without finalizing (empty segment)
            drop(self.segment_sink);
            None
        };

        Ok(FlushedSession {
            ctx: self.ctx,
            wal: self.wal,
            finished_segment,
            pending_deletes: self.pending_deletes,
        })
    }

    /// Abort the session without committing.
    pub fn abort(self) -> Result<()> {
        // Drop everything without finalizing
        drop(self.wal);
        drop(self.segment_sink);
        Ok(())
    }
}

/// Phase 2: Segment file on disk, WAL not yet synced.
/// No `add_document()` method — documents are sealed.
pub struct FlushedSession {
    pub ctx: WriterContext,
    pub wal: Box<dyn WriteAheadLog>,
    pub finished_segment: Option<Arc<Segment>>,
    pub pending_deletes: Vec<DocId>,
}

impl FlushedSession {
    /// WAL sync + MVCC snapshot creation. Consumes self.
    pub fn commit(mut self) -> Result<CommittedSession> {
        self.wal.append(Operation::Commit)?;
        self.wal.sync()?;

        // Update MVCC snapshot
        let mut segments = self.ctx.mvcc.current_snapshot().segments.clone();
        if let Some(seg) = self.finished_segment {
            let docs_committed = seg.doc_count as usize;
            segments.push(seg);

            // Handle pending deletes
            let current_snapshot = self.ctx.mvcc.current_snapshot();
            let mut deleted_docs = (*current_snapshot.deleted_docs).clone();
            for doc_id in &self.pending_deletes {
                deleted_docs.insert(doc_id.0 as u32);
            }

            let snapshot = self.ctx.mvcc.create_snapshot_with_deletes(
                segments,
                std::sync::Arc::new(deleted_docs),
            );

            Ok(CommittedSession {
                snapshot_version: snapshot.version,
                docs_committed,
            })
        } else {
            // No documents written — still update snapshot for deletes
            let current_snapshot = self.ctx.mvcc.current_snapshot();
            let mut deleted_docs = (*current_snapshot.deleted_docs).clone();
            for doc_id in &self.pending_deletes {
                deleted_docs.insert(doc_id.0 as u32);
            }

            let snapshot = if !self.pending_deletes.is_empty() {
                self.ctx.mvcc.create_snapshot_with_deletes(
                    segments,
                    std::sync::Arc::new(deleted_docs),
                )
            } else {
                self.ctx.mvcc.current_snapshot()
            };

            Ok(CommittedSession {
                snapshot_version: snapshot.version,
                docs_committed: 0,
            })
        }
    }

    /// Abort without syncing WAL or publishing MVCC snapshot.
    pub fn abort(self) -> Result<()> {
        drop(self.wal);
        Ok(())
    }
}

/// Phase 3: MVCC snapshot published. Immutable receipt.
pub struct CommittedSession {
    pub snapshot_version: u64,
    pub docs_committed: usize,
}
