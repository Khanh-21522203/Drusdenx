## Reader Pool And MVCC Snapshots

### Purpose

Provide consistent read views over immutable snapshot versions while writes continue concurrently.

### Scope

**In scope:**
- MVCC snapshot/version lifecycle.
- Reader pool caching by snapshot version.
- Snapshot-based segment reader construction.
- Deleted-document filtering at read time.

**Out of scope:**
- Query parsing/planning rules.
- Segment writing and WAL mutation flow.
- User transaction operation buffering.

### Primary User Flow

1. Caller issues a search.
2. Reader pool gets current snapshot from MVCC.
3. Reader pool reuses or opens segment readers for that snapshot version.
4. Search execution scans segments and filters deleted docs from snapshot bitmap.
5. Caller receives results consistent with the captured snapshot.

### System Flow

1. Entry point: `src/reader/reader_pool.rs:ReaderPool::get_reader`.
2. Reader pool reads `MVCCController::current_snapshot`.
3. Existing `IndexReader` for version is reused from `reader_cache`; otherwise created via `create_reader_for_snapshot`.
4. Each segment is opened as `SegmentReader` and cached by `(version, segment_index)`; failures are logged and counted.
5. Query execution calls `SegmentSearch::search` on segment readers and then removes docs in `deleted_docs` bitmap.

```
ReaderPool::get_reader
  └── MVCCController::current_snapshot
        ├── [cached version] -> reuse IndexReader
        └── [cache miss]
              ├── open SegmentReader per segment
              └── cache IndexReader + segment readers
```

### Data Model

- `MVCCController` fields: `versions (BTreeMap<u64, Snapshot>)`, `leases (BTreeMap<u64, Weak<SnapshotLease>>)`, `active_txns`, `current_version`, `max_versions`.
- `Snapshot` fields: `version`, `segments (Vec<Arc<Segment>>)` , `timestamp`, `doc_count`, `deleted_docs (Arc<RoaringBitmap>)`, `_lease`.
- `IndexReader` fields: `snapshot`, `segments (Vec<Arc<RwLock<SegmentReader>>>)`, `deleted_docs`, `index`.
- `ReaderPool` cache fields: `reader_cache` and `segment_reader_cache`.
- `ReaderPool` also tracks `segment_open_failures` as an atomic counter.
- Persistence rule: snapshots/readers are in-memory read views over persisted segment files.

### Interfaces and Contracts

- `MVCCController::create_snapshot(segments) -> Arc<Snapshot>`.
- `MVCCController::create_snapshot_with_deletes(segments, deleted_docs) -> Arc<Snapshot>`.
- `MVCCController::current_snapshot() -> Arc<Snapshot>`.
- `ReaderPool::get_reader() -> Result<Arc<IndexReader>>`.
- `IndexReader::search(query) -> Result<SearchResults>` and `search_with_limit(query, limit)`.
- `SnapshotReader::new(snapshot, storage, index)` provides explicit per-snapshot reader wrapper.
- `ReadGuard<R: SegmentRead>` provides generic RAII guard over snapshot + segment readers.

### Dependencies

**Internal modules:**
- `src/mvcc/controller.rs` — versioning and snapshot control.
- `src/storage/segment_reader.rs` — per-segment reads.
- `src/query/matcher.rs` — segment-level query matching.
- `src/search/results.rs` — result shape.

**External services/libraries:**
- `roaring` — deleted-document bitmap.
- `parking_lot` — read/write locks for pool-held readers.

### Failure Modes and Edge Cases

- Segment open failures inside reader-pool snapshot construction are skipped but now explicitly logged and counted.
- MVCC GC now treats strong-count `> 1` as pinned (map-owned lease + active guards) and evicts unpinned old versions predictably when version limits are exceeded.
- `MVCCController::begin_transaction` derives transaction ID from `current_version`, so concurrent begin calls at same version can collide.
- Reader cache cleanup uses simple oldest-version eviction and removes related segment-reader cache entries.

### Observability and Debugging

- Start at `ReaderPool::get_reader` and `create_reader_for_snapshot` to inspect cache-hit vs open behavior.
- Use snapshot `version` and `deleted_docs.len()` when checking stale-read or deletion-visibility issues.
- `DatabaseStats::reader_segment_open_failures` and `health_check().checks["ReaderPool"]` expose accumulated segment-open failures.

### Risks and Notes

- Snapshot semantics are central to correctness; segment-open failure counts and lease-aware GC behavior now provide basic operational signals for stale/partial read risks.
- The `index` field in `IndexReader` can diverge from per-segment persisted index readers depending on write/read wiring.

Changes:
