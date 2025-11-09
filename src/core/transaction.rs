use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashMap;
use parking_lot::RwLock;
use crate::core::types::{Document, DocId};
use crate::core::error::{Result, Error, ErrorKind};
use crate::mvcc::controller::{MVCCController, Snapshot, IsolationLevel};

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
    pub read_set: Arc<RwLock<HashMap<DocId, u64>>>,  // Track reads for validation
    pub write_set: Arc<RwLock<HashMap<DocId, Document>>>,  // Track writes
    mvcc: Arc<MVCCController>,  // Use MVCC directly instead of Database
}

impl Transaction {
    /// Begin new transaction
    pub fn begin(mvcc: Arc<MVCCController>, isolation_level: IsolationLevel) -> Self {
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
            mvcc,
        }
    }
    
    /// Read document within transaction
    pub fn read(&self, doc_id: DocId) -> Result<Option<Document>> {
        // Check if we're still active
        if *self.state.read() != TransactionState::Active {
            return Err(Error::new(ErrorKind::InvalidState, "Transaction is not active".to_string()));
        }
        
        // First check write set (read your own writes)
        if let Some(doc) = self.write_set.read().get(&doc_id) {
            return Ok(Some(doc.clone()));
        }
        
        // Track read for validation
        self.read_set.write().insert(doc_id, self.snapshot.version);
        
        // Read from snapshot (MVCC provides isolation)
        // Check if document is deleted in this snapshot
        if self.snapshot.deleted_docs.contains(doc_id.0 as u32) {
            return Ok(None);
        }
        
        // Search through segments in the snapshot for the document
        // TODO: This is a simplified implementation - a real system would
        // use an index to quickly locate documents
        for _segment in &self.snapshot.segments {
            // In a real implementation, we would:
            // 1. Open a SegmentReader for the segment
            // 2. Search for the document by ID
            // 3. Return it if found
            // For now, return None as segments don't have doc ID index
        }
        
        Ok(None)
    }
    
    /// Insert document in transaction
    pub fn insert(&self, doc: Document) -> Result<()> {
        self.check_active()?;
        
        // Add to write set
        self.write_set.write().insert(doc.id, doc.clone());
        
        // Log operation
        self.operations.lock().unwrap().push(TransactionOp::Insert(doc));
        
        Ok(())
    }
    
    /// Update document in transaction
    pub fn update(&self, doc_id: DocId, doc: Document) -> Result<()> {
        self.check_active()?;
        
        // Add to write set
        self.write_set.write().insert(doc_id, doc.clone());
        
        // Log operation
        self.operations.lock().unwrap().push(TransactionOp::Update(doc_id, doc));
        
        Ok(())
    }
    
    /// Delete document in transaction
    pub fn delete(&self, doc_id: DocId) -> Result<()> {
        self.check_active()?;
        
        // Remove from write set if present
        self.write_set.write().remove(&doc_id);
        
        // Log operation
        self.operations.lock().unwrap().push(TransactionOp::Delete(doc_id));
        
        Ok(())
    }
    
    /// Commit transaction with 2-phase commit
    /// Returns the list of operations to be executed
    pub fn commit(&self) -> Result<Vec<TransactionOp>> {
        // Phase 1: Prepare
        {
            let mut state = self.state.write();
            if *state != TransactionState::Active {
                return Err(Error::new(ErrorKind::InvalidState, "Transaction is not active".to_string()));
            }
            *state = TransactionState::Preparing;
        }
        
        // Validate read set (optimistic concurrency control)
        if self.isolation_level != IsolationLevel::ReadCommitted {
            if !self.validate_reads()? {
                self.abort()?;
                return Err(Error::new(ErrorKind::InvalidState, "Transaction validation failed".to_string()));
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
        
        // Check if any read documents were modified since our snapshot
        for (_doc_id, version) in self.read_set.read().iter() {
            if *version != current_snapshot.version {
                // Version has changed, validation fails
                return Ok(false);
            }
        }
        
        Ok(true)
    }
    
    fn check_active(&self) -> Result<()> {
        if *self.state.read() != TransactionState::Active {
            return Err(Error::new(ErrorKind::InvalidState, "Transaction is not active".to_string()));
        }
        Ok(())
    }
}

/// Transaction manager for coordinating transactions
pub struct TransactionManager {
    active_transactions: Arc<RwLock<HashMap<u64, Arc<Transaction>>>>,
    mvcc: Arc<MVCCController>,
}

impl TransactionManager {
    pub fn new(mvcc: Arc<MVCCController>) -> Self {
        TransactionManager {
            active_transactions: Arc::new(RwLock::new(HashMap::new())),
            mvcc,
        }
    }
    
    /// Begin new transaction
    pub fn begin_transaction(&self, isolation_level: IsolationLevel) -> Arc<Transaction> {
        let tx = Arc::new(Transaction::begin(self.mvcc.clone(), isolation_level));
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