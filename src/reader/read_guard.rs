use crate::core::error::Result;
use crate::mvcc::controller::Snapshot;
use crate::query::ast::Query;
use crate::query::matcher::DocumentMatcher;
use crate::reader::segment_opener::SegmentRead;
use crate::search::results::ScoredDocument;
use std::sync::Arc;

/// RAII guard holding a consistent read view.
///
/// The `snapshot` field contains an `Arc<SnapshotLease>` via the `Snapshot` struct.
/// As long as this guard is alive, the MVCC controller cannot GC the snapshot version.
/// Dropping the last `ReadGuard` (and therefore the last `Snapshot` clone) makes the
/// version eligible for GC on the next `MVCCController::gc()` call.
pub struct ReadGuard<R: SegmentRead> {
    pub snapshot: Arc<Snapshot>,
    pub segments: Vec<R>,
}

impl<R: SegmentRead> ReadGuard<R> {
    pub fn new(snapshot: Arc<Snapshot>, segments: Vec<R>) -> Self {
        ReadGuard { snapshot, segments }
    }

    pub fn version(&self) -> u64 {
        self.snapshot.version
    }

    pub fn search(
        &self,
        query: &Query,
        matcher: &DocumentMatcher,
    ) -> Result<Vec<ScoredDocument>> {
        let mut all_results = Vec::new();
        for seg in &self.segments {
            let results = seg.search(query, matcher)?;
            all_results.extend(results);
        }
        Ok(all_results)
    }
}
