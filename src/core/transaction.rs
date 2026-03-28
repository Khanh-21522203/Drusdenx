use crate::core::error::{Error, ErrorKind, Result};
use crate::core::types::{DocId, Document};
use crate::mvcc::controller::{IsolationLevel, MVCCController, Snapshot};
use crate::storage::layout::StorageLayout;
use crate::storage::segment::{SegmentHeader, SegmentId};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Transaction ID generator
static TRANSACTION_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Transaction state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransactionState {
    Active,
    Preparing,
    Committed,
    Aborted,
}

/// Transaction operation log
#[derive(Debug, Clone)]
pub enum TransactionOp {
    Insert(Document),
    Update(DocId, Document),
    Delete(DocId),
}

/// ACID Transaction implementation
pub struct Transaction {
    pub id: u64,
    pub isolation_level: IsolationLevel,
    pub state: Arc<RwLock<TransactionState>>,
    pub operations: Arc<Mutex<Vec<TransactionOp>>>,
    pub snapshot: Arc<Snapshot>,
    pub read_set: Arc<RwLock<HashMap<DocId, u64>>>, // Track reads for validation
    pub write_set: Arc<RwLock<HashMap<DocId, Document>>>, // Track writes
    storage: Arc<StorageLayout>,
    mvcc: Arc<MVCCController>, // Use MVCC directly instead of Database
}

impl Transaction {
    /// Begin new transaction
    pub fn begin(
        mvcc: Arc<MVCCController>,
        storage: Arc<StorageLayout>,
        isolation_level: IsolationLevel,
    ) -> Self {
        let id = TRANSACTION_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        let snapshot = mvcc.current_snapshot();

        Transaction {
            id,
            isolation_level,
            state: Arc::new(RwLock::new(TransactionState::Active)),
            operations: Arc::new(Mutex::new(Vec::new())),
            snapshot,
            read_set: Arc::new(RwLock::new(HashMap::new())),
            write_set: Arc::new(RwLock::new(HashMap::new())),
            storage,
            mvcc,
        }
    }

    /// Read document within transaction
    pub fn read(&self, doc_id: DocId) -> Result<Option<Document>> {
        // Check if we're still active
        if *self.state.read() != TransactionState::Active {
            return Err(Error::new(
                ErrorKind::InvalidState,
                "Transaction is not active".to_string(),
            ));
        }

        // First check write set (read your own writes)
        if let Some(doc) = self.write_set.read().get(&doc_id) {
            return Ok(Some(doc.clone()));
        }

        // Track read for validation
        self.read_set.write().insert(doc_id, self.snapshot.version);

        self.read_from_snapshot(&self.snapshot, doc_id)
    }

    /// Insert document in transaction
    pub fn insert(&self, doc: Document) -> Result<()> {
        self.check_active()?;

        // Add to write set
        self.write_set.write().insert(doc.id, doc.clone());

        // Log operation
        self.operations
            .lock()
            .unwrap()
            .push(TransactionOp::Insert(doc));

        Ok(())
    }

    /// Update document in transaction
    pub fn update(&self, doc_id: DocId, doc: Document) -> Result<()> {
        self.check_active()?;

        // Add to write set
        self.write_set.write().insert(doc_id, doc.clone());

        // Log operation
        self.operations
            .lock()
            .unwrap()
            .push(TransactionOp::Update(doc_id, doc));

        Ok(())
    }

    /// Delete document in transaction
    pub fn delete(&self, doc_id: DocId) -> Result<()> {
        self.check_active()?;

        // Remove from write set if present
        self.write_set.write().remove(&doc_id);

        // Log operation
        self.operations
            .lock()
            .unwrap()
            .push(TransactionOp::Delete(doc_id));

        Ok(())
    }

    /// Commit transaction with 2-phase commit
    /// Returns the list of operations to be executed
    pub fn commit(&self) -> Result<Vec<TransactionOp>> {
        // Phase 1: Prepare
        {
            let mut state = self.state.write();
            if *state != TransactionState::Active {
                return Err(Error::new(
                    ErrorKind::InvalidState,
                    "Transaction is not active".to_string(),
                ));
            }
            *state = TransactionState::Preparing;
        }

        // Validate read set (optimistic concurrency control)
        if self.isolation_level != IsolationLevel::ReadCommitted {
            if !self.validate_reads()? {
                self.abort()?;
                return Err(Error::new(
                    ErrorKind::InvalidState,
                    "Transaction validation failed".to_string(),
                ));
            }
        }

        // Phase 2: Commit
        // Return operations to be executed by the database
        let operations = self.operations.lock().unwrap().clone();

        // Mark transaction as committed
        *self.state.write() = TransactionState::Committed;

        Ok(operations)
    }

    /// Rollback/abort transaction
    pub fn rollback(&self) -> Result<()> {
        self.abort()
    }

    fn abort(&self) -> Result<()> {
        *self.state.write() = TransactionState::Aborted;

        // Clear all in-memory state
        self.operations.lock().unwrap().clear();
        self.read_set.write().clear();
        self.write_set.write().clear();

        Ok(())
    }

    /// Validate that read set hasn't changed
    fn validate_reads(&self) -> Result<bool> {
        let current_snapshot = self.mvcc.current_snapshot();

        if current_snapshot.version == self.snapshot.version {
            return Ok(true);
        }

        let mut tracked_doc_ids: HashSet<DocId> = self.read_set.read().keys().copied().collect();
        tracked_doc_ids.extend(self.write_set.read().keys().copied());
        for op in self.operations.lock().unwrap().iter() {
            match op {
                TransactionOp::Insert(doc) => {
                    tracked_doc_ids.insert(doc.id);
                }
                TransactionOp::Update(doc_id, _) | TransactionOp::Delete(doc_id) => {
                    tracked_doc_ids.insert(*doc_id);
                }
            }
        }

        for doc_id in tracked_doc_ids {
            let original_doc = self.read_from_snapshot(&self.snapshot, doc_id)?;
            let current_doc = self.read_from_snapshot(&current_snapshot, doc_id)?;
            if !Self::documents_equal(&original_doc, &current_doc) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn read_from_snapshot(&self, snapshot: &Snapshot, doc_id: DocId) -> Result<Option<Document>> {
        if snapshot.deleted_docs.contains(doc_id.0 as u32) {
            return Ok(None);
        }

        // Search newest segments first so later writes shadow older copies.
        for segment in snapshot.segments.iter().rev() {
            if let Some(doc) = self.read_document_from_segment(segment.id, doc_id)? {
                return Ok(Some(doc));
            }
        }

        Ok(None)
    }

    fn read_document_from_segment(
        &self,
        segment_id: SegmentId,
        doc_id: DocId,
    ) -> Result<Option<Document>> {
        let path = self.storage.segment_path(&segment_id);
        let mut file = File::open(path)?;
        let header: SegmentHeader = bincode::deserialize_from(&mut file)?;

        for _ in 0..header.doc_count {
            let mut len_buf = [0u8; 4];
            file.read_exact(&mut len_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;

            let mut block_buf = vec![0u8; len];
            file.read_exact(&mut block_buf)?;
            let compressed: crate::compression::compress::CompressedBlock =
                bincode::deserialize(&block_buf)?;
            let raw = compressed.decompress()?;
            let doc: Document = bincode::deserialize(&raw)?;

            if doc.id == doc_id {
                return Ok(Some(doc));
            }
        }

        Ok(None)
    }

    fn documents_equal(left: &Option<Document>, right: &Option<Document>) -> bool {
        match (left, right) {
            (Some(a), Some(b)) => a.id == b.id && a.fields == b.fields,
            (None, None) => true,
            _ => false,
        }
    }

    fn check_active(&self) -> Result<()> {
        if *self.state.read() != TransactionState::Active {
            return Err(Error::new(
                ErrorKind::InvalidState,
                "Transaction is not active".to_string(),
            ));
        }
        Ok(())
    }
}

/// Transaction manager for coordinating transactions
pub struct TransactionManager {
    active_transactions: Arc<RwLock<HashMap<u64, Arc<Transaction>>>>,
    mvcc: Arc<MVCCController>,
    storage: Arc<StorageLayout>,
}

impl TransactionManager {
    pub fn new(mvcc: Arc<MVCCController>, storage: Arc<StorageLayout>) -> Self {
        TransactionManager {
            active_transactions: Arc::new(RwLock::new(HashMap::new())),
            mvcc,
            storage,
        }
    }

    /// Begin new transaction
    pub fn begin_transaction(&self, isolation_level: IsolationLevel) -> Arc<Transaction> {
        let tx = Arc::new(Transaction::begin(
            self.mvcc.clone(),
            self.storage.clone(),
            isolation_level,
        ));
        self.active_transactions.write().insert(tx.id, tx.clone());
        tx
    }

    /// Get active transaction by ID
    pub fn get_transaction(&self, tx_id: u64) -> Option<Arc<Transaction>> {
        self.active_transactions.read().get(&tx_id).cloned()
    }

    /// Clean up completed transactions
    pub fn cleanup(&self) {
        let mut transactions = self.active_transactions.write();
        transactions.retain(|_, tx| {
            let state = *tx.state.read();
            state == TransactionState::Active || state == TransactionState::Preparing
        });
    }

    /// Get transaction statistics
    pub fn stats(&self) -> TransactionStats {
        let transactions = self.active_transactions.read();
        let mut active = 0;
        let mut preparing = 0;

        for tx in transactions.values() {
            match *tx.state.read() {
                TransactionState::Active => active += 1,
                TransactionState::Preparing => preparing += 1,
                _ => {}
            }
        }

        TransactionStats {
            total: transactions.len(),
            active,
            preparing,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransactionStats {
    pub total: usize,
    pub active: usize,
    pub preparing: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyzer::Analyzer;
    use crate::compression::compress::CompressionType;
    use crate::core::config::MergePolicyType;
    use crate::core::types::FieldValue;
    use crate::memory::buffer_pool::BufferPool;
    use crate::memory::pool::MemoryPool;
    use crate::parallel::indexer::ParallelIndexer;
    use crate::storage::layout::StorageLayout;
    use crate::writer::index_writer::IndexWriter;
    use std::collections::HashMap;

    fn make_writer(storage: Arc<StorageLayout>, mvcc: Arc<MVCCController>) -> IndexWriter {
        IndexWriter::new_with_merge_policy(
            storage,
            mvcc,
            MemoryPool::new(8, 1024 * 1024),
            Arc::new(BufferPool::new(4 * 1024 * 1024)),
            Arc::new(ParallelIndexer::new(2)),
            Arc::new(Analyzer::standard_english()),
            MergePolicyType::Tiered,
            CompressionType::LZ4,
        )
        .unwrap()
    }

    fn make_doc(id: u64, value: &str) -> Document {
        Document {
            id: DocId(id),
            fields: HashMap::from([("content".to_string(), FieldValue::Text(value.to_string()))]),
        }
    }

    #[test]
    fn transaction_read_fetches_document_from_snapshot_segments() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(StorageLayout::new(temp_dir.path().to_path_buf()).unwrap());
        let mvcc = Arc::new(MVCCController::new());
        let mut writer = make_writer(storage.clone(), mvcc.clone());

        let doc = make_doc(1, "hello");
        writer.add_document(doc.clone()).unwrap();
        writer.commit().unwrap();

        let tx = Transaction::begin(mvcc, storage, IsolationLevel::RepeatableRead);
        let read_back = tx.read(doc.id).unwrap().unwrap();

        assert_eq!(read_back.id, doc.id);
        assert_eq!(read_back.fields, doc.fields);
    }

    #[test]
    fn transaction_commit_detects_conflicting_write_on_same_document() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(StorageLayout::new(temp_dir.path().to_path_buf()).unwrap());
        let mvcc = Arc::new(MVCCController::new());
        let mut writer = make_writer(storage.clone(), mvcc.clone());

        let doc = make_doc(1, "before");
        writer.add_document(doc.clone()).unwrap();
        writer.commit().unwrap();

        let tx = Transaction::begin(mvcc.clone(), storage.clone(), IsolationLevel::Serializable);
        tx.update(doc.id, make_doc(1, "tx-update")).unwrap();

        writer.delete_document(doc.id).unwrap();
        writer.commit().unwrap();

        let commit_result = tx.commit();
        assert!(commit_result.is_err());
    }
}
