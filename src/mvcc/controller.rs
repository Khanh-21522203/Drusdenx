use crate::core::error::Result;
use crate::core::types::{DocId, Document};
use crate::mvcc::snapshot::{SnapshotLease, Version};
use crate::storage::segment::Segment;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use roaring::RoaringBitmap;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

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
    /// Lease map: version → Weak<SnapshotLease>
    /// GC only evicts a version when the Weak cannot be upgraded (strong_count == 0)
    leases: Arc<RwLock<BTreeMap<u64, Weak<SnapshotLease>>>>,
    pub active_txns: Arc<RwLock<HashSet<TxId>>>,
    pub current_version: Arc<AtomicU64>,
    pub max_versions: usize,
}

/// Snapshot of index at a point in time.
/// Holds an Arc<SnapshotLease> so that GC cannot evict this version
/// while any Snapshot clone is live.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub version: u64,
    pub segments: Vec<Arc<Segment>>,
    pub timestamp: DateTime<Utc>,
    pub doc_count: usize,
    pub deleted_docs: Arc<RoaringBitmap>,
    /// RAII pin: while this Arc lives, MVCCController cannot GC this version
    _lease: Arc<SnapshotLease>,
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
            leases: Arc::new(RwLock::new(BTreeMap::new())),
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
        deleted_docs: Arc<RoaringBitmap>,
    ) -> Arc<Snapshot> {
        let version = self.current_version.fetch_add(1, Ordering::SeqCst);

        // Calculate total doc count
        let doc_count = segments.iter().map(|s| s.doc_count as usize).sum();

        // Create a fresh lease Arc for this version
        let lease = Arc::new(SnapshotLease {
            version: Version(version),
        });
        let weak_lease = Arc::downgrade(&lease);

        let snapshot = Arc::new(Snapshot {
            version,
            segments,
            timestamp: Utc::now(),
            doc_count,
            deleted_docs,
            _lease: lease,
        });

        // Store weak lease reference first so GC can reason about pinning.
        {
            let mut leases = self.leases.write();
            leases.insert(version, weak_lease);
        }

        {
            let mut versions = self.versions.write();
            versions.insert(version, (*snapshot).clone());
            // GC old versions (using lease-aware GC)
            self.gc_old_versions_leased(&mut versions);
        }

        snapshot
    }

    pub fn current_snapshot(&self) -> Arc<Snapshot> {
        let versions = self.versions.read();
        let current = self.current_version.load(Ordering::Acquire);

        // fetch_add returns old value, so current snapshot is at (current - 1)
        // unless current is 0 (no snapshots created yet)
        let snapshot_version = if current > 0 { current - 1 } else { 0 };

        versions
            .get(&snapshot_version)
            .map(|s| Arc::new(s.clone()))
            .unwrap_or_else(|| Arc::new(Snapshot::default()))
    }

    /// Safe GC that uses Weak<SnapshotLease> to detect live snapshots.
    /// A version is eligible for eviction only when its Weak::strong_count() == 0.
    pub fn gc(&self) {
        let mut versions = self.versions.write();
        self.gc_old_versions_leased(&mut versions);
    }

    fn gc_old_versions_leased(&self, versions: &mut BTreeMap<u64, Snapshot>) {
        if versions.len() <= self.max_versions {
            return;
        }

        let latest_version = self
            .current_version
            .load(Ordering::Acquire)
            .saturating_sub(1);
        let mut leases = self.leases.write();

        while versions.len() > self.max_versions {
            let mut removed_any = false;
            for version in versions.keys().copied().collect::<Vec<_>>() {
                if version == latest_version {
                    continue;
                }

                // One strong reference is owned by the snapshot stored in `versions`.
                // Additional strong references indicate active snapshot guards.
                let is_pinned = leases
                    .get(&version)
                    .map(|weak| weak.strong_count() > 1)
                    .unwrap_or(false);

                if !is_pinned {
                    versions.remove(&version);
                    leases.remove(&version);
                    removed_any = true;
                    break;
                }
            }

            if !removed_any {
                break;
            }
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
        // Default snapshot has no lease — it's a placeholder
        let lease = Arc::new(SnapshotLease {
            version: Version(0),
        });
        Snapshot {
            version: 0,
            segments: Vec::new(),
            timestamp: Utc::now(),
            doc_count: 0,
            deleted_docs: Arc::new(RoaringBitmap::new()),
            _lease: lease,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_keeps_pinned_versions_and_evicts_unpinned_old_versions() {
        let mut mvcc = MVCCController::new();
        mvcc.max_versions = 2;

        let pinned = mvcc.create_snapshot(Vec::new()); // version 0
        let _ = mvcc.create_snapshot(Vec::new()); // version 1
        let _ = mvcc.create_snapshot(Vec::new()); // version 2, triggers GC

        let versions = mvcc.versions.read();
        assert!(versions.contains_key(&0), "pinned version should remain");
        assert!(versions.contains_key(&2), "latest version should remain");
        assert!(
            !versions.contains_key(&1),
            "old unpinned version should be evicted"
        );
        drop(versions);

        drop(pinned); // release guard for version 0
        let _ = mvcc.create_snapshot(Vec::new()); // version 3, triggers GC again

        let versions = mvcc.versions.read();
        assert!(
            !versions.contains_key(&0),
            "unpinned old version should be evicted"
        );
        assert!(versions.contains_key(&2));
        assert!(versions.contains_key(&3));
    }
}
