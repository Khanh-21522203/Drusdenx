use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::{BTreeMap, HashSet};
use roaring::RoaringBitmap;
use std::sync::Arc;
use chrono::{DateTime, Utc};
use crate::core::types::{DocId, Document};
use crate::storage::segment::Segment;
use crate::core::error::Result;

/// Transaction ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TxId(pub u64);

impl TxId {
    pub fn new(id: u64) -> Self {
        TxId(id)
    }
}

/// Write operation
#[derive(Debug, Clone)]
pub enum Operation {
    AddDocument(Document),
    DeleteDocument(DocId),
    UpdateDocument { id: DocId, doc: Document },
}

/// Multi-Version Concurrency Control
pub struct MVCCController {
    pub versions: Arc<RwLock<BTreeMap<u64, Snapshot>>>,
    pub active_txns: Arc<RwLock<HashSet<TxId>>>,
    pub current_version: Arc<AtomicU64>,
    pub max_versions: usize,
}

/// Snapshot of index at a point in time
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub version: u64,
    pub segments: Vec<Arc<Segment>>,
    pub timestamp: DateTime<Utc>,
    pub doc_count: usize,
    pub deleted_docs: Arc<RoaringBitmap>,
}

/// Transaction for write operations
pub struct Transaction {
    pub id: TxId,
    pub snapshot: Arc<Snapshot>,
    pub operations: Vec<Operation>,
    pub isolation_level: IsolationLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl MVCCController {
    pub fn new() -> Self {
        MVCCController {
            versions: Arc::new(RwLock::new(BTreeMap::new())),
            active_txns: Arc::new(RwLock::new(HashSet::new())),
            current_version: Arc::new(AtomicU64::new(0)),
            max_versions: 100,
        }
    }

    pub fn create_snapshot(&self, segments: Vec<Arc<Segment>>) -> Arc<Snapshot> {
        self.create_snapshot_with_deletes(segments, Arc::new(RoaringBitmap::new()))
    }
    
    /// Create snapshot with specific deleted docs bitmap
    pub fn create_snapshot_with_deletes(
        &self, 
        segments: Vec<Arc<Segment>>, 
        deleted_docs: Arc<RoaringBitmap>
    ) -> Arc<Snapshot> {
        let version = self.current_version.fetch_add(1, Ordering::SeqCst);
        
        // Calculate total doc count
        let doc_count = segments.iter()
            .map(|s| s.doc_count as usize)
            .sum();

        let snapshot = Arc::new(Snapshot {
            version,
            segments,
            timestamp: Utc::now(),
            doc_count,
            deleted_docs,
        });

        let mut versions = self.versions.write();
        versions.insert(version, (*snapshot).clone());

        // GC old versions
        self.gc_old_versions(&mut versions);

        snapshot
    }

    pub fn current_snapshot(&self) -> Arc<Snapshot> {
        let versions = self.versions.read();
        let current = self.current_version.load(Ordering::Acquire);
        
        // fetch_add returns old value, so current snapshot is at (current - 1)
        // unless current is 0 (no snapshots created yet)
        let snapshot_version = if current > 0 { current - 1 } else { 0 };
        
        versions.get(&snapshot_version)
            .map(|s| Arc::new(s.clone()))
            .unwrap_or_else(|| Arc::new(Snapshot::default()))
    }

    fn gc_old_versions(&self, versions: &mut BTreeMap<u64, Snapshot>) {
        if versions.len() > self.max_versions {
            // Get min_active then drop lock before retain()
            let min_active = {
                let active_txns = self.active_txns.read();
                active_txns.iter().map(|tx| tx.0).min().unwrap_or(u64::MAX)
            };

            let min_keep = self.max_versions / 2;
            let current_len = versions.len();

            // Remove versions older than oldest active transaction
            versions.retain(|&v, _| v >= min_active || current_len <= min_keep);
        }
    }

    pub fn begin_transaction(&self, isolation: IsolationLevel) -> Transaction {
        let tx_id = TxId::new(self.current_version.load(Ordering::Acquire));
        self.active_txns.write().insert(tx_id);

        Transaction {
            id: tx_id,
            snapshot: self.current_snapshot(),
            operations: Vec::new(),
            isolation_level: isolation,
        }
    }

    pub fn commit_transaction(&self, tx: Transaction) -> Result<()> {
        self.active_txns.write().remove(&tx.id);
        Ok(())
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            version: 0,
            segments: Vec::new(),
            timestamp: Utc::now(),
            doc_count: 0,
            deleted_docs: Arc::new(RoaringBitmap::new()),
        }
    }
}